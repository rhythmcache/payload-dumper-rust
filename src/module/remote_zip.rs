use crate::module::http::HttpReader;
use anyhow::{Result, anyhow};
use std::io::{self, Read, Seek, SeekFrom};

pub struct RemoteZipReader {
    pub http_reader: HttpReader,
    payload_offset: u64,
    payload_size: u64,
    current_position: u64,
}

impl RemoteZipReader {
    fn find_payload_via_metadata(http_reader: &mut HttpReader) -> Result<(u64, u64)> {
        let search_size = std::cmp::min(http_reader.content_length, 131072);
        http_reader.seek(SeekFrom::End(-(search_size as i64)))?;

        let mut tail_buffer = vec![0u8; search_size as usize];
        let bytes_read = http_reader.read(&mut tail_buffer)?;
        tail_buffer.truncate(bytes_read);

        let search_pattern = b"payload.bin:";
        for i in 0..tail_buffer.len().saturating_sub(search_pattern.len()) {
            if tail_buffer[i..].starts_with(search_pattern) {
                let line_end = i + tail_buffer[i..]
                    .iter()
                    .position(|&b| b == b',')
                    .unwrap_or_else(|| tail_buffer[i..].len());

                let property_line = &tail_buffer[i..line_end];
                let values: Vec<&[u8]> = property_line.split(|&b| b == b':').collect();

                if values.len() >= 3 {
                    if let (Ok(offset_str), Ok(size_str)) = (
                        std::str::from_utf8(values[1]),
                        std::str::from_utf8(values[2]),
                    ) {
                        if let (Ok(offset), Ok(size)) =
                            (offset_str.parse::<u64>(), size_str.parse::<u64>())
                        {
                            http_reader.seek(SeekFrom::Start(offset))?;
                            let mut header = [0u8; 4];
                            http_reader.read_exact(&mut header)?;

                            if header[0] == 0x50 && header[1] == 0x4B {
                                return Ok((offset, size));
                            }
                        }
                    }
                }
            }
        }
        Err(anyhow!("Could not find payload.bin metadata"))
    }

    fn find_eocd(reader: &mut HttpReader) -> Result<(u64, u16)> {
        let max_comment_size = 65535;
        let eocd_min_size = 22;
        let max_search = std::cmp::min(
            reader.content_length,
            (max_comment_size + eocd_min_size) as u64,
        );
        let chunk_size = 8192;
        let mut current_pos = reader.content_length;
        let mut eocd_pos = None;
        let mut buffer = vec![0u8; chunk_size];

        while current_pos > reader.content_length.saturating_sub(max_search) && eocd_pos.is_none() {
            let read_size = std::cmp::min(
                chunk_size,
                (current_pos - reader.content_length.saturating_sub(max_search)) as usize,
            );
            let read_pos = current_pos.saturating_sub(read_size as u64);

            reader.seek(SeekFrom::Start(read_pos))?;
            let bytes_read = reader.read(&mut buffer[..read_size])?;

            if bytes_read == 0 {
                break;
            }
            for i in (0..bytes_read - 3).rev() {
                if buffer[i] == 0x50
                    && buffer[i + 1] == 0x4B
                    && buffer[i + 2] == 0x05
                    && buffer[i + 3] == 0x06
                {
                    eocd_pos = Some(read_pos + i as u64);
                    break;
                }
            }

            current_pos = read_pos;
            if current_pos > 3 {
                current_pos -= 3;
            }
        }

        let eocd_offset =
            eocd_pos.ok_or_else(|| anyhow!("Could not find End of Central Directory record"))?;
        reader.seek(SeekFrom::Start(eocd_offset + 10))?;
        let mut num_entries_buf = [0u8; 2];
        reader.read_exact(&mut num_entries_buf)?;
        let num_entries = u16::from_le_bytes([num_entries_buf[0], num_entries_buf[1]]);

        Ok((eocd_offset, num_entries))
    }

    fn find_payload_via_zip_structure(mut http_reader: HttpReader) -> Result<Self> {
        let (eocd_offset, num_entries) = Self::find_eocd(&mut http_reader)?;
        http_reader.seek(SeekFrom::Start(eocd_offset + 16))?;
        let mut cd_offset_buf = [0u8; 4];
        http_reader.read_exact(&mut cd_offset_buf)?;
        let cd_offset = u32::from_le_bytes([
            cd_offset_buf[0],
            cd_offset_buf[1],
            cd_offset_buf[2],
            cd_offset_buf[3],
        ]) as u64;
        let (real_cd_offset, real_num_entries) = if cd_offset == 0xFFFFFFFF {
            let mut found_locator = false;
            let mut zip64_eocd_offset = 0u64;
            let search_start = if eocd_offset > 20 {
                eocd_offset - 20
            } else {
                0
            };

            http_reader.seek(SeekFrom::Start(search_start))?;
            let mut buffer = vec![0u8; (eocd_offset - search_start) as usize];
            http_reader.read_exact(&mut buffer)?;

            for i in (0..buffer.len() - 3).rev() {
                if buffer[i] == 0x50
                    && buffer[i + 1] == 0x4B
                    && buffer[i + 2] == 0x06
                    && buffer[i + 3] == 0x07
                {
                    found_locator = true;
                    if i + 8 + 8 <= buffer.len() {
                        zip64_eocd_offset = u64::from_le_bytes([
                            buffer[i + 8],
                            buffer[i + 9],
                            buffer[i + 10],
                            buffer[i + 11],
                            buffer[i + 12],
                            buffer[i + 13],
                            buffer[i + 14],
                            buffer[i + 15],
                        ]);
                    }
                    break;
                }
            }

            if !found_locator {
                return Err(anyhow!(
                    "ZIP64 format indicated but ZIP64 EOCD locator not found"
                ));
            }

            http_reader.seek(SeekFrom::Start(zip64_eocd_offset))?;
            let mut zip64_eocd = [0u8; 56];
            http_reader.read_exact(&mut zip64_eocd)?;

            if zip64_eocd[0] != 0x50
                || zip64_eocd[1] != 0x4B
                || zip64_eocd[2] != 0x06
                || zip64_eocd[3] != 0x06
            {
                return Err(anyhow!("Invalid ZIP64 EOCD signature"));
            }

            let cd_offset = u64::from_le_bytes([
                zip64_eocd[48],
                zip64_eocd[49],
                zip64_eocd[50],
                zip64_eocd[51],
                zip64_eocd[52],
                zip64_eocd[53],
                zip64_eocd[54],
                zip64_eocd[55],
            ]);

            let num_entries = u64::from_le_bytes([
                zip64_eocd[32],
                zip64_eocd[33],
                zip64_eocd[34],
                zip64_eocd[35],
                zip64_eocd[36],
                zip64_eocd[37],
                zip64_eocd[38],
                zip64_eocd[39],
            ]);

            (cd_offset, num_entries as usize)
        } else {
            (cd_offset as u64, num_entries as usize)
        };

        http_reader.seek(SeekFrom::Start(real_cd_offset))?;

        for _entry_num in 0..real_num_entries {
            let mut entry_header = [0u8; 46];
            http_reader.read_exact(&mut entry_header)?;

            if entry_header[0] != 0x50
                || entry_header[1] != 0x4B
                || entry_header[2] != 0x01
                || entry_header[3] != 0x02
            {
                return Err(anyhow!("Invalid central directory header signature"));
            }

            let filename_len = u16::from_le_bytes([entry_header[28], entry_header[29]]) as usize;
            let extra_len = u16::from_le_bytes([entry_header[30], entry_header[31]]) as usize;
            let comment_len = u16::from_le_bytes([entry_header[32], entry_header[33]]) as usize;

            let mut local_header_offset = u32::from_le_bytes([
                entry_header[42],
                entry_header[43],
                entry_header[44],
                entry_header[45],
            ]) as u64;

            let mut compressed_size = u32::from_le_bytes([
                entry_header[20],
                entry_header[21],
                entry_header[22],
                entry_header[23],
            ]) as u64;

            let mut filename = vec![0u8; filename_len];
            http_reader.read_exact(&mut filename)?;

            let mut extra_data = vec![0u8; extra_len];
            http_reader.read_exact(&mut extra_data)?;

            http_reader.seek(SeekFrom::Current(comment_len as i64))?;

            if local_header_offset == 0xFFFFFFFF || compressed_size == 0xFFFFFFFF {
                let mut pos = 0;
                while pos + 4 <= extra_data.len() {
                    let header_id = u16::from_le_bytes([extra_data[pos], extra_data[pos + 1]]);
                    let data_size =
                        u16::from_le_bytes([extra_data[pos + 2], extra_data[pos + 3]]) as usize;

                    if header_id == 0x0001 && pos + 4 + data_size <= extra_data.len() {
                        let mut field_pos = pos + 4;

                        if local_header_offset == 0xFFFFFFFF && field_pos + 8 <= pos + 4 + data_size
                        {
                            local_header_offset = u64::from_le_bytes([
                                extra_data[field_pos],
                                extra_data[field_pos + 1],
                                extra_data[field_pos + 2],
                                extra_data[field_pos + 3],
                                extra_data[field_pos + 4],
                                extra_data[field_pos + 5],
                                extra_data[field_pos + 6],
                                extra_data[field_pos + 7],
                            ]);
                            field_pos += 8;
                        }

                        if compressed_size == 0xFFFFFFFF && field_pos + 8 <= pos + 4 + data_size {
                            compressed_size = u64::from_le_bytes([
                                extra_data[field_pos],
                                extra_data[field_pos + 1],
                                extra_data[field_pos + 2],
                                extra_data[field_pos + 3],
                                extra_data[field_pos + 4],
                                extra_data[field_pos + 5],
                                extra_data[field_pos + 6],
                                extra_data[field_pos + 7],
                            ]);
                        }
                    }
                    pos += 4 + data_size;
                }
            }

            if filename == b"payload.bin" || filename.ends_with(b"/payload.bin") {
                http_reader.seek(SeekFrom::Start(local_header_offset))?;
                let mut local_header = [0u8; 30];
                http_reader.read_exact(&mut local_header)?;

                if local_header[0] != 0x50
                    || local_header[1] != 0x4B
                    || local_header[2] != 0x03
                    || local_header[3] != 0x04
                {
                    return Err(anyhow!("Invalid local file header signature"));
                }

                let local_filename_len =
                    u16::from_le_bytes([local_header[26], local_header[27]]) as u64;
                let local_extra_len =
                    u16::from_le_bytes([local_header[28], local_header[29]]) as u64;

                let payload_offset =
                    local_header_offset + 30 + local_filename_len + local_extra_len;
                http_reader.seek(SeekFrom::Start(payload_offset))?;
                let mut magic = [0u8; 4];
                http_reader.read_exact(&mut magic)?;

                if magic != *b"CrAU" {
                    return Err(anyhow!(
                        "Invalid payload file: magic 'CrAU' not found at calculated offset"
                    ));
                }

                http_reader.seek(SeekFrom::Start(payload_offset))?;

                return Ok(Self {
                    http_reader,
                    payload_offset,
                    payload_size: compressed_size,
                    current_position: 0,
                });
            }
        }

        Err(anyhow!("Could not find payload.bin in ZIP file"))
    }

    pub fn new_for_parallel(url: String) -> Result<Self> {
        let http_reader = HttpReader::new_silent(url.clone())?;
        if let Ok(payload_info) = Self::find_payload_via_metadata(&mut http_reader.clone()) {
            return Ok(Self {
                http_reader,
                payload_offset: payload_info.0,
                payload_size: payload_info.1,
                current_position: 0,
            });
        }
        Self::find_payload_via_zip_structure(http_reader)
    }
}

impl Read for RemoteZipReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.current_position >= self.payload_size {
            return Ok(0);
        }

        let remaining = self.payload_size - self.current_position;
        let to_read = std::cmp::min(buf.len() as u64, remaining) as usize;

        if to_read == 0 {
            return Ok(0);
        }

        self.http_reader
            .seek(SeekFrom::Start(self.payload_offset + self.current_position))?;

        let bytes_read = self.http_reader.read(&mut buf[..to_read])?;

        self.current_position += bytes_read as u64;

        Ok(bytes_read)
    }
}

impl Seek for RemoteZipReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                if offset >= 0 {
                    self.current_position.saturating_add(offset as u64)
                } else {
                    self.current_position.saturating_sub(offset.abs() as u64)
                }
            }
            SeekFrom::End(offset) => {
                if offset >= 0 {
                    self.payload_size.saturating_add(offset as u64)
                } else {
                    self.payload_size.saturating_sub(offset.abs() as u64)
                }
            }
        };
        if new_pos > self.payload_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Invalid position to seek: {} in size {}",
                    new_pos, self.payload_size
                ),
            ));
        }

        self.current_position = new_pos;
        Ok(self.current_position)
    }
}
