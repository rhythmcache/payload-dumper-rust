use crate::zip::zip_io::ZipIO;
use anyhow::{Result, anyhow};

// ZIP signatures
pub const LOCAL_FILE_HEADER_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
pub const CENTRAL_DIR_HEADER_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
pub const EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
pub const ZIP64_EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x06, 0x06];
pub const ZIP64_EOCD_LOCATOR_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x06, 0x07];

#[derive(Debug, Clone)]
pub struct ZipEntry {
    pub name: String,
    pub uncompressed_size: u64,
    pub offset: u64,
    pub compression_method: u16,
}

pub struct ZipParser;

impl ZipParser {
    /// find End of Central Directory record
    pub async fn find_eocd<I: ZipIO>(io: &I) -> Result<(u64, u16)> {
        let file_size = io.size().await?;

        let max_comment_size = 65535;
        let eocd_min_size = 22;
        let max_search = std::cmp::min(file_size, (max_comment_size + eocd_min_size) as u64);
        let chunk_size = 8192;
        let mut current_pos = file_size;
        let mut eocd_pos = None;
        let mut buffer = vec![0u8; chunk_size];

        while current_pos > file_size.saturating_sub(max_search) && eocd_pos.is_none() {
            let read_size = std::cmp::min(
                chunk_size,
                (current_pos - file_size.saturating_sub(max_search)) as usize,
            );
            let read_pos = current_pos.saturating_sub(read_size as u64);

            io.read_at(read_pos, &mut buffer[..read_size]).await?;

            if read_size >= 4 {
                for i in (0..=read_size - 4).rev() {
                    if buffer[i..i + 4] == EOCD_SIGNATURE {
                        eocd_pos = Some(read_pos + i as u64);
                        break;
                    }
                }
            }

            current_pos = read_pos;
            if current_pos > 3 {
                current_pos -= 3;
            }
        }

        let eocd_offset =
            eocd_pos.ok_or_else(|| anyhow!("Could not find End of Central Directory record"))?;

        // read number of entries
        let mut num_entries_buf = [0u8; 2];
        io.read_at(eocd_offset + 10, &mut num_entries_buf).await?;
        let num_entries = u16::from_le_bytes(num_entries_buf);

        Ok((eocd_offset, num_entries))
    }

    /// read ZIP64 EOCD information
    pub async fn read_zip64_eocd<I: ZipIO>(io: &I, eocd_offset: u64) -> Result<(u64, u64)> {
        if eocd_offset < 20 {
            return Err(anyhow!("Invalid ZIP64 structure"));
        }

        let search_start = eocd_offset.saturating_sub(20);
        let mut buffer = vec![0u8; (eocd_offset - search_start) as usize];
        io.read_at(search_start, &mut buffer).await?;

        let mut zip64_eocd_offset = 0u64;
        let mut found_locator = false;

        if buffer.len() >= 4 {
            for i in (0..=buffer.len() - 4).rev() {
                if buffer[i..i + 4] == ZIP64_EOCD_LOCATOR_SIGNATURE {
                    found_locator = true;
                    if i + 16 <= buffer.len() {
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
        }

        if !found_locator {
            return Err(anyhow!(
                "ZIP64 format indicated but ZIP64 EOCD locator not found"
            ));
        }

        let mut zip64_eocd = [0u8; 56];
        io.read_at(zip64_eocd_offset, &mut zip64_eocd).await?;

        if zip64_eocd[0..4] != ZIP64_EOCD_SIGNATURE {
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

        Ok((cd_offset, num_entries))
    }

    /// get central directory offset and number of entries
    pub async fn get_central_directory_info<I: ZipIO>(io: &I) -> Result<(u64, usize)> {
        let (eocd_offset, num_entries) = Self::find_eocd(io).await?;

        let mut cd_offset_buf = [0u8; 4];
        io.read_at(eocd_offset + 16, &mut cd_offset_buf).await?;
        let cd_offset = u32::from_le_bytes(cd_offset_buf) as u64;

        if cd_offset == 0xFFFFFFFF {
            let (real_cd_offset, real_num_entries) = Self::read_zip64_eocd(io, eocd_offset).await?;
            Ok((real_cd_offset, real_num_entries as usize))
        } else {
            Ok((cd_offset, num_entries as usize))
        }
    }

    /// read a single central directory entry
    pub async fn read_central_directory_entry<I: ZipIO>(
        io: &I,
        offset: u64,
    ) -> Result<(ZipEntry, u64)> {
        let mut entry_header = [0u8; 46];
        io.read_at(offset, &mut entry_header).await?;

        if entry_header[0..4] != CENTRAL_DIR_HEADER_SIGNATURE {
            return Err(anyhow!("Invalid central directory header signature"));
        }

        let compression_method = u16::from_le_bytes([entry_header[10], entry_header[11]]);
        let filename_len = u16::from_le_bytes([entry_header[28], entry_header[29]]) as usize;
        let extra_len = u16::from_le_bytes([entry_header[30], entry_header[31]]) as usize;
        let comment_len = u16::from_le_bytes([entry_header[32], entry_header[33]]) as usize;

        let mut local_header_offset = u32::from_le_bytes([
            entry_header[42],
            entry_header[43],
            entry_header[44],
            entry_header[45],
        ]) as u64;

        let mut uncompressed_size = u32::from_le_bytes([
            entry_header[24],
            entry_header[25],
            entry_header[26],
            entry_header[27],
        ]) as u64;

        // read filename
        let mut filename = vec![0u8; filename_len];
        io.read_at(offset + 46, &mut filename).await?;

        // read extra data
        let mut extra_data = vec![0u8; extra_len];
        io.read_at(offset + 46 + filename_len as u64, &mut extra_data)
            .await?;

        // handle ZIP64 extra fields
        if local_header_offset == 0xFFFFFFFF || uncompressed_size == 0xFFFFFFFF {
            let mut pos = 0;
            while pos + 4 <= extra_data.len() {
                let header_id = u16::from_le_bytes([extra_data[pos], extra_data[pos + 1]]);
                let data_size =
                    u16::from_le_bytes([extra_data[pos + 2], extra_data[pos + 3]]) as usize;

                if header_id == 0x0001 && pos + 4 + data_size <= extra_data.len() {
                    let mut field_pos = pos + 4;

                    if uncompressed_size == 0xFFFFFFFF && field_pos + 8 <= pos + 4 + data_size {
                        uncompressed_size = u64::from_le_bytes([
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

                    if local_header_offset == 0xFFFFFFFF && field_pos + 8 <= pos + 4 + data_size {
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
                    }
                    break;
                }
                pos += 4 + data_size;
            }
        }

        let next_offset = offset + 46 + filename_len as u64 + extra_len as u64 + comment_len as u64;

        Ok((
            ZipEntry {
                name: String::from_utf8_lossy(&filename).into_owned(),
                uncompressed_size,
                offset: local_header_offset,
                compression_method,
            },
            next_offset,
        ))
    }

    /// find payload.bin entry in ZIP central directory
    pub async fn find_payload_entry<I: ZipIO>(io: &I) -> Result<ZipEntry> {
        let (cd_offset, num_entries) = Self::get_central_directory_info(io).await?;
        let mut current_offset = cd_offset;

        for _ in 0..num_entries {
            let (entry, next_offset) =
                Self::read_central_directory_entry(io, current_offset).await?;
            current_offset = next_offset;

            // check compression method - we only support stored (uncompressed)
            if entry.compression_method != 0 {
                continue;
            }

            // look for payload.bin at root level
            if entry.name == "payload.bin" || entry.name.ends_with("/payload.bin") {
                return Ok(entry);
            }
        }

        Err(anyhow!(
            "Could not find uncompressed payload.bin in ZIP file"
        ))
    }

    /// calculate the actual data offset for a ZIP entry (after local header)
    pub async fn get_data_offset<I: ZipIO>(io: &I, entry: &ZipEntry) -> Result<u64> {
        let mut local_header = [0u8; 30];
        io.read_at(entry.offset, &mut local_header).await?;

        if local_header[0..4] != LOCAL_FILE_HEADER_SIGNATURE {
            return Err(anyhow!("Invalid local file header signature"));
        }

        // double-check compression method in local header
        let local_compression = u16::from_le_bytes([local_header[8], local_header[9]]);
        if local_compression != 0 {
            return Err(anyhow!(
                "payload.bin is compressed, expected uncompressed (STORED)"
            ));
        }

        let local_filename_len = u16::from_le_bytes([local_header[26], local_header[27]]) as u64;
        let local_extra_len = u16::from_le_bytes([local_header[28], local_header[29]]) as u64;

        let data_offset = entry.offset + 30 + local_filename_len + local_extra_len;
        Ok(data_offset)
    }

    /// verify payload magic at given offset
    pub async fn verify_payload_magic<I: ZipIO>(io: &I, offset: u64) -> Result<()> {
        let mut magic = [0u8; 4];
        io.read_at(offset, &mut magic).await?;

        if &magic != b"CrAU" {
            return Err(anyhow!(
                "Invalid payload file: magic 'CrAU' not found at calculated offset"
            ));
        }

        Ok(())
    }
}
