use crate::module::http::HttpReader;
use crate::module::zip::zip_core::ZipParser;
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

        let search_pattern = b"payload.bin";
        for i in 0..tail_buffer.len().saturating_sub(search_pattern.len()) {
            if tail_buffer[i..].starts_with(search_pattern) {
                if let Some(colon_pos) = tail_buffer[i..].iter().position(|&b| b == b':') {
                    let start = i + colon_pos + 1;
                    if let Some(end) = tail_buffer[start..]
                        .iter()
                        .position(|&b| b == b',' || b == b'\n' || b == b'\r')
                    {
                        let metadata_str = std::str::from_utf8(&tail_buffer[start..start + end])
                            .map_err(|_| anyhow!("Invalid UTF-8 in metadata"))?;
                        let parts: Vec<&str> = metadata_str.split(':').collect();

                        if parts.len() >= 2 {
                            if let (Ok(offset), Ok(size)) =
                                (parts[0].parse::<u64>(), parts[1].parse::<u64>())
                            {
                                http_reader.seek(SeekFrom::Start(offset))?;
                                let mut header = [0u8; 4];
                                http_reader.read_exact(&mut header)?;

                                if header[0..2] == [0x50, 0x4B] {
                                    return Ok((offset, size));
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(anyhow!("Could not find payload.bin metadata"))
    }

    fn find_payload_via_zip_structure(mut http_reader: HttpReader) -> Result<Self> {
        // Find payload.bin entry using shared ZIP parsing logic
        let entry = ZipParser::find_payload_entry(&mut http_reader)?;

        // Calculate data offset
        let payload_offset = ZipParser::get_data_offset(&mut http_reader, &entry)?;

        // Verify payload magic
        ZipParser::verify_payload_magic(&mut http_reader, payload_offset)?;

        // Reset position
        http_reader.seek(SeekFrom::Start(payload_offset))?;

        Ok(Self {
            http_reader,
            payload_offset,
            payload_size: entry.compressed_size, // Since we only support stored method
            current_position: 0,
        })
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

        // Use read_at instead of seek + read
        let bytes_read = self.http_reader.read_at(
            self.payload_offset + self.current_position,
            &mut buf[..to_read],
        )?;

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
