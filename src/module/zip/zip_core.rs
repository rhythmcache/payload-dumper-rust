use crate::module::zip::advanced_reader::AdvancedReader;
use anyhow::{Result, anyhow};
use std::io::{Read, Seek, SeekFrom};

// ZIP signatures
pub const LOCAL_FILE_HEADER_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
pub const CENTRAL_DIR_HEADER_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
pub const EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
pub const ZIP64_EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x06, 0x06];
pub const ZIP64_EOCD_LOCATOR_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x06, 0x07];

#[derive(Debug, Clone)]
pub struct ZipEntry {
    pub name: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub offset: u64,
    pub compression_method: u16,
    pub data_offset: u64,
}

pub struct ZipParser;

impl ZipParser {
    /// Find End of Central Directory record in a ZIP file
    pub fn find_eocd<R: Read + Seek + ?Sized>(reader: &mut R) -> Result<(u64, u16)> {
        // Get file size
        let file_size = reader.seek(SeekFrom::End(0))?;

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

            reader.seek(SeekFrom::Start(read_pos))?;
            let bytes_read = reader.read(&mut buffer[..read_size])?;

            if bytes_read == 0 {
                break;
            }

            if bytes_read >= 4 {
                for i in (0..=bytes_read - 4).rev() {
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

        // Read number of entries
        reader.seek(SeekFrom::Start(eocd_offset + 10))?;
        let mut num_entries_buf = [0u8; 2];
        reader.read_exact(&mut num_entries_buf)?;
        let num_entries = u16::from_le_bytes(num_entries_buf);

        Ok((eocd_offset, num_entries))
    }

    /// Optimized EOCD finder for memory-mapped readers
    pub fn find_eocd_optimized<R: AdvancedReader + ?Sized>(reader: &mut R) -> Result<(u64, u16)> {
        let file_size = reader.size()?;

        // For memory-mapped readers, we can search more efficiently
        if reader.supports_zero_copy() {
            let max_comment_size = 65535;
            let eocd_min_size = 22;
            let max_search = std::cmp::min(file_size, (max_comment_size + eocd_min_size) as u64);
            let search_start = file_size.saturating_sub(max_search);
            let search_len = (file_size - search_start) as usize;

            if let Some(data) = reader.get_slice(search_start, search_len) {
                // Search backwards through memory for EOCD signature
                for i in (0..=data.len().saturating_sub(4)).rev() {
                    if data[i..i + 4] == EOCD_SIGNATURE {
                        let eocd_offset = search_start + i as u64;

                        // Read number of entries from the EOCD
                        if i + 14 < data.len() {
                            let num_entries = u16::from_le_bytes([data[i + 10], data[i + 11]]);
                            return Ok((eocd_offset, num_entries));
                        }
                    }
                }

                return Err(anyhow!("Could not find End of Central Directory record"));
            }
        }

        // Fallback to traditional method
        Self::find_eocd(reader)
    }

    /// Read ZIP64 EOCD information
    pub fn read_zip64_eocd<R: Read + Seek + ?Sized>(
        reader: &mut R,
        eocd_offset: u64,
    ) -> Result<(u64, u64)> {
        // Look for ZIP64 EOCD locator
        if eocd_offset < 20 {
            return Err(anyhow!("Invalid ZIP64 structure"));
        }

        let search_start = if eocd_offset > 20 {
            eocd_offset - 20
        } else {
            0
        };

        reader.seek(SeekFrom::Start(search_start))?;
        let mut buffer = vec![0u8; (eocd_offset - search_start) as usize];
        reader.read_exact(&mut buffer)?;

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

        // Read ZIP64 EOCD
        reader.seek(SeekFrom::Start(zip64_eocd_offset))?;
        let mut zip64_eocd = [0u8; 56];
        reader.read_exact(&mut zip64_eocd)?;

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

    /// Get central directory offset and number of entries
    pub fn get_central_directory_info<R: Read + Seek + ?Sized>(
        reader: &mut R,
    ) -> Result<(u64, usize)> {
        let (eocd_offset, num_entries) = Self::find_eocd(reader)?;

        reader.seek(SeekFrom::Start(eocd_offset + 16))?;
        let mut cd_offset_buf = [0u8; 4];
        reader.read_exact(&mut cd_offset_buf)?;
        let cd_offset = u32::from_le_bytes(cd_offset_buf) as u64;

        if cd_offset == 0xFFFFFFFF {
            let (real_cd_offset, real_num_entries) = Self::read_zip64_eocd(reader, eocd_offset)?;
            Ok((real_cd_offset, real_num_entries as usize))
        } else {
            Ok((cd_offset, num_entries as usize))
        }
    }

    /// Optimized version for AdvancedReader
    pub fn get_central_directory_info_optimized<R: AdvancedReader + ?Sized>(
        reader: &mut R,
    ) -> Result<(u64, usize)> {
        let (eocd_offset, num_entries) = Self::find_eocd_optimized(reader)?;

        // Try to read CD offset using zero-copy if available
        if reader.supports_zero_copy() {
            if let Some(eocd_data) = reader.get_slice(eocd_offset, 22) {
                let cd_offset = u32::from_le_bytes([
                    eocd_data[16],
                    eocd_data[17],
                    eocd_data[18],
                    eocd_data[19],
                ]) as u64;

                if cd_offset == 0xFFFFFFFF {
                    let (real_cd_offset, real_num_entries) =
                        Self::read_zip64_eocd(reader, eocd_offset)?;
                    return Ok((real_cd_offset, real_num_entries as usize));
                } else {
                    return Ok((cd_offset, num_entries as usize));
                }
            }
        }

        // Fallback to traditional method
        Self::get_central_directory_info(reader)
    }

    /// Read a single central directory entry
    pub fn read_central_directory_entry<R: Read + Seek + ?Sized>(
        reader: &mut R,
    ) -> Result<ZipEntry> {
        let mut entry_header = [0u8; 46];
        reader.read_exact(&mut entry_header)?;

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

        let mut compressed_size = u32::from_le_bytes([
            entry_header[20],
            entry_header[21],
            entry_header[22],
            entry_header[23],
        ]) as u64;

        let mut uncompressed_size = u32::from_le_bytes([
            entry_header[24],
            entry_header[25],
            entry_header[26],
            entry_header[27],
        ]) as u64;

        // Read filename
        let mut filename = vec![0u8; filename_len];
        reader.read_exact(&mut filename)?;

        // Read extra data
        let mut extra_data = vec![0u8; extra_len];
        reader.read_exact(&mut extra_data)?;

        // Skip comment
        reader.seek(SeekFrom::Current(comment_len as i64))?;

        // Handle ZIP64 extra fields
        if local_header_offset == 0xFFFFFFFF
            || compressed_size == 0xFFFFFFFF
            || uncompressed_size == 0xFFFFFFFF
        {
            let mut pos = 0;
            while pos + 4 <= extra_data.len() {
                let header_id = u16::from_le_bytes([extra_data[pos], extra_data[pos + 1]]);
                let data_size =
                    u16::from_le_bytes([extra_data[pos + 2], extra_data[pos + 3]]) as usize;

                if header_id == 0x0001 && pos + 4 + data_size <= extra_data.len() {
                    let mut field_pos = pos + 4;

                    // Read ZIP64 fields in order: uncompressed_size, compressed_size, local_header_offset
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

        Ok(ZipEntry {
            name: String::from_utf8_lossy(&filename).into_owned(),
            compressed_size,
            uncompressed_size,
            offset: local_header_offset,
            compression_method,
            data_offset: 0, // Will be calculated when needed
        })
    }

    /// Optimized entry parsing from memory slice (for memory-mapped readers)
    pub fn parse_entry_from_slice(data: &[u8], offset: &mut usize) -> Result<ZipEntry> {
        if data.len() < *offset + 46 {
            return Err(anyhow!("Insufficient data for central directory entry"));
        }

        let entry_header = &data[*offset..*offset + 46];

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

        let mut compressed_size = u32::from_le_bytes([
            entry_header[20],
            entry_header[21],
            entry_header[22],
            entry_header[23],
        ]) as u64;

        let mut uncompressed_size = u32::from_le_bytes([
            entry_header[24],
            entry_header[25],
            entry_header[26],
            entry_header[27],
        ]) as u64;

        *offset += 46;

        // Read filename
        if data.len() < *offset + filename_len {
            return Err(anyhow!("Insufficient data for filename"));
        }
        let filename = &data[*offset..*offset + filename_len];
        *offset += filename_len;

        // Read extra data
        if data.len() < *offset + extra_len {
            return Err(anyhow!("Insufficient data for extra field"));
        }
        let extra_data = &data[*offset..*offset + extra_len];
        *offset += extra_len;

        // Skip comment
        *offset += comment_len;

        // Handle ZIP64 extra fields
        if local_header_offset == 0xFFFFFFFF
            || compressed_size == 0xFFFFFFFF
            || uncompressed_size == 0xFFFFFFFF
        {
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

        Ok(ZipEntry {
            name: String::from_utf8_lossy(filename).into_owned(),
            compressed_size,
            uncompressed_size,
            offset: local_header_offset,
            compression_method,
            data_offset: 0,
        })
    }

    /// Find payload.bin entry in ZIP central directory (optimized)
    pub fn find_payload_entry<R: Read + Seek>(reader: &mut R) -> Result<ZipEntry> {
        let (cd_offset, num_entries) = Self::get_central_directory_info(reader)?;
        reader.seek(SeekFrom::Start(cd_offset))?;

        for _entry_num in 0..num_entries {
            let entry = Self::read_central_directory_entry(reader)?;

            // Check compression method - we only support stored (uncompressed)
            if entry.compression_method != 0 {
                continue;
            }

            // Look for payload.bin at root level
            if entry.name == "payload.bin" || entry.name.ends_with("/payload.bin") {
                return Ok(entry);
            }
        }

        Err(anyhow!("Could not find payload.bin in ZIP file"))
    }

    /// Calculate the actual data offset for a ZIP entry (after local header)
    pub fn get_data_offset<R: Read + Seek + ?Sized>(
        reader: &mut R,
        entry: &ZipEntry,
    ) -> Result<u64> {
        reader.seek(SeekFrom::Start(entry.offset))?;
        let mut local_header = [0u8; 30];
        reader.read_exact(&mut local_header)?;

        if local_header[0..4] != LOCAL_FILE_HEADER_SIGNATURE {
            return Err(anyhow!("Invalid local file header signature"));
        }

        // Double-check compression method in local header
        let local_compression = u16::from_le_bytes([local_header[8], local_header[9]]);
        if local_compression != 0 {
            return Err(anyhow!("payload.bin is compressed, expected uncompressed"));
        }

        let local_filename_len = u16::from_le_bytes([local_header[26], local_header[27]]) as u64;
        let local_extra_len = u16::from_le_bytes([local_header[28], local_header[29]]) as u64;

        let data_offset = entry.offset + 30 + local_filename_len + local_extra_len;
        Ok(data_offset)
    }

    /// Optimized data offset calculation for AdvancedReader
    pub fn get_data_offset_optimized<R: AdvancedReader + ?Sized>(
        reader: &mut R,
        entry: &ZipEntry,
    ) -> Result<u64> {
        // Try zero-copy approach first
        if reader.supports_zero_copy() {
            if let Some(header_data) = reader.get_slice(entry.offset, 30) {
                if header_data[0..4] != LOCAL_FILE_HEADER_SIGNATURE {
                    return Err(anyhow!("Invalid local file header signature"));
                }

                // Double-check compression method in local header
                let local_compression = u16::from_le_bytes([header_data[8], header_data[9]]);
                if local_compression != 0 {
                    return Err(anyhow!("payload.bin is compressed, expected uncompressed"));
                }

                let local_filename_len =
                    u16::from_le_bytes([header_data[26], header_data[27]]) as u64;
                let local_extra_len = u16::from_le_bytes([header_data[28], header_data[29]]) as u64;

                let data_offset = entry.offset + 30 + local_filename_len + local_extra_len;
                return Ok(data_offset);
            }
        }

        // Fallback to traditional method
        Self::get_data_offset(reader, entry)
    }

    /// Verify payload magic at given offset
    pub fn verify_payload_magic<R: Read + Seek>(reader: &mut R, offset: u64) -> Result<()> {
        reader.seek(SeekFrom::Start(offset))?;
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;

        if &magic != b"CrAU" {
            return Err(anyhow!(
                "Invalid payload file: magic 'CrAU' not found at calculated offset"
            ));
        }

        Ok(())
    }
}
