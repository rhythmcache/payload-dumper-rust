// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use crate::constants::*;
use crate::zip::zip_io::ZipIO;
use anyhow::{Result, anyhow};

#[derive(Debug, Clone)]
pub struct ZipEntry {
    pub name: String,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub offset: u64,
    pub compression_method: u16,
}

#[derive(Debug, Clone)]
pub struct ZipMetadataInfo {
    pub entry_name: String,
    pub header_offset: u64,
    pub payload_data_offset: u64,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub compression_method: u16,
    pub total_entries: Option<usize>,
    pub central_directory_offset: Option<u64>,
    pub archive_size: u64,
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

        // read filename
        let mut filename = vec![0u8; filename_len];
        io.read_at(offset + 46, &mut filename).await?;

        // read extra data
        let mut extra_data = vec![0u8; extra_len];
        io.read_at(offset + 46 + filename_len as u64, &mut extra_data)
            .await?;

        // handle ZIP64 extra fields
        if local_header_offset == 0xFFFFFFFF
            || uncompressed_size == 0xFFFFFFFF
            || compressed_size == 0xFFFFFFFF
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

        let next_offset = offset + 46 + filename_len as u64 + extra_len as u64 + comment_len as u64;

        Ok((
            ZipEntry {
                name: String::from_utf8_lossy(&filename).into_owned(),
                uncompressed_size,
                compressed_size,
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

        if &magic != PAYLOAD_MAGIC {
            return Err(anyhow!(
                "Invalid payload file: magic 'CrAU' not found at calculated offset"
            ));
        }

        Ok(())
    }

    /// fast path: check for Android OTA metadata at the beginning of the ZIP archive.
    /// Android OTA packages place uncompressed `META-INF/com/android/metadata`
    /// at the start of the archive, containing exact `payload.bin:<offset>:<size>` mappings.
    pub async fn try_fast_ota_metadata<I: ZipIO>(io: &I) -> Result<Option<ZipMetadataInfo>> {
        let file_size = io.size().await?;
        if file_size < 128 {
            return Ok(None);
        }

        // Read initial chunk (up to 16 KB) to inspect local file headers at the start
        let initial_chunk_size = std::cmp::min(file_size, 16384) as usize;
        let mut buffer = vec![0u8; initial_chunk_size];
        io.read_at(0, &mut buffer).await?;

        let mut pos = 0usize;
        while pos + 30 <= buffer.len() {
            if buffer[pos..pos + 4] != LOCAL_FILE_HEADER_SIGNATURE {
                break;
            }

            let flags = u16::from_le_bytes([buffer[pos + 6], buffer[pos + 7]]);
            let compression_method = u16::from_le_bytes([buffer[pos + 8], buffer[pos + 9]]);
            let compressed_size = u32::from_le_bytes([
                buffer[pos + 18],
                buffer[pos + 19],
                buffer[pos + 20],
                buffer[pos + 21],
            ]) as usize;
            let uncompressed_size = u32::from_le_bytes([
                buffer[pos + 22],
                buffer[pos + 23],
                buffer[pos + 24],
                buffer[pos + 25],
            ]) as usize;
            let filename_len = u16::from_le_bytes([buffer[pos + 26], buffer[pos + 27]]) as usize;
            let extra_len = u16::from_le_bytes([buffer[pos + 28], buffer[pos + 29]]) as usize;

            let name_start = match pos.checked_add(30) {
                Some(s) => s,
                None => break,
            };
            let name_end = match name_start.checked_add(filename_len) {
                Some(e) => e,
                None => break,
            };
            if name_end > buffer.len() {
                break;
            }

            let filename = String::from_utf8_lossy(&buffer[name_start..name_end]);
            let normalized_name = filename.trim_start_matches("./").trim_start_matches('/');

            if normalized_name == "META-INF/com/android/metadata" && compression_method == 0 {
                // Reject unusually large metadata files (> 1 MB) to prevent unbounded allocations
                const MAX_METADATA_SIZE: usize = 1024 * 1024;
                if uncompressed_size > MAX_METADATA_SIZE || compressed_size > MAX_METADATA_SIZE {
                    return Ok(None);
                }

                let content_start = match name_end.checked_add(extra_len) {
                    Some(s) => s,
                    None => return Ok(None),
                };
                let content_end = match content_start.checked_add(uncompressed_size) {
                    Some(e) => e,
                    None => return Ok(None),
                };

                let metadata_bytes = if content_end <= buffer.len() {
                    buffer[content_start..content_end].to_vec()
                } else {
                    if content_end as u64 > file_size {
                        return Ok(None);
                    }
                    let mut full_meta = vec![0u8; uncompressed_size];
                    io.read_at(content_start as u64, &mut full_meta).await?;
                    full_meta
                };

                let metadata_str = String::from_utf8_lossy(&metadata_bytes);

                let mut payload_offset: Option<u64> = None;
                let mut payload_size: Option<u64> = None;

                for line in metadata_str.lines() {
                    let line = line.trim();
                    let props = if let Some(stripped) =
                        line.strip_prefix("ota-streaming-property-files=")
                    {
                        stripped
                    } else if let Some(stripped) = line.strip_prefix("ota-property-files=") {
                        stripped
                    } else {
                        continue;
                    };

                    for item in props.split(',') {
                        let item = item.trim();
                        let parts: Vec<&str> = item.split(':').collect();
                        if parts.len() >= 3
                            && parts[0].trim() == "payload.bin"
                            && let (Ok(offset), Ok(size)) =
                                (parts[1].parse::<u64>(), parts[2].parse::<u64>())
                        {
                            payload_offset = Some(offset);
                            payload_size = Some(size);
                            break;
                        }
                    }

                    if payload_offset.is_some() {
                        break;
                    }
                }

                if let (Some(offset), Some(size)) = (payload_offset, payload_size) {
                    // Bounds validation: payload must not overflow or exceed archive bounds
                    let fits_archive = offset
                        .checked_add(size)
                        .map(|end| end <= file_size)
                        .unwrap_or(false);
                    let magic_fits = offset
                        .checked_add(4)
                        .map(|end| end <= file_size)
                        .unwrap_or(false);

                    if fits_archive && magic_fits {
                        // Read up to 128 bytes before payload_offset plus 4 magic bytes
                        let check_start = offset.saturating_sub(128);
                        let check_len = match (offset - check_start).checked_add(4) {
                            Some(len) => len as usize,
                            None => return Ok(None),
                        };
                        let mut check_buf = vec![0u8; check_len];
                        if io.read_at(check_start, &mut check_buf).await.is_err() {
                            return Ok(None);
                        }

                        let magic_pos = (offset - check_start) as usize;
                        if &check_buf[magic_pos..magic_pos + 4] != PAYLOAD_MAGIC {
                            return Ok(None);
                        }

                        // Verify the local file header of payload.bin and ensure compression_method == 0 (STORED)
                        let mut verified_header_offset = None;
                        for i in 0..magic_pos.saturating_sub(29) {
                            if check_buf[i..i + 4] == LOCAL_FILE_HEADER_SIGNATURE {
                                let comp_method =
                                    u16::from_le_bytes([check_buf[i + 8], check_buf[i + 9]]);
                                let fn_len =
                                    u16::from_le_bytes([check_buf[i + 26], check_buf[i + 27]])
                                        as usize;
                                let ex_len =
                                    u16::from_le_bytes([check_buf[i + 28], check_buf[i + 29]])
                                        as usize;

                                if comp_method == 0
                                    && fn_len == 11
                                    && i + 30 + fn_len <= magic_pos
                                    && &check_buf[i + 30..i + 30 + fn_len] == b"payload.bin"
                                {
                                    let cand_hdr = check_start + i as u64;
                                    if cand_hdr + 30 + 11 + ex_len as u64 == offset {
                                        // Once verified that this header leads directly to offset:
                                        // If bit 3 is clear and size is not ZIP64 (0xFFFFFFFF), verify size matches
                                        let entry_flags = u16::from_le_bytes([
                                            check_buf[i + 6],
                                            check_buf[i + 7],
                                        ]);
                                        let local_u_size = u32::from_le_bytes([
                                            check_buf[i + 22],
                                            check_buf[i + 23],
                                            check_buf[i + 24],
                                            check_buf[i + 25],
                                        ]);
                                        if (entry_flags & 0x0008) == 0
                                            && local_u_size != 0xFFFF_FFFF
                                            && local_u_size as u64 != size
                                        {
                                            return Ok(None);
                                        }

                                        verified_header_offset = Some(cand_hdr);
                                        break;
                                    }
                                }
                            }
                        }

                        let header_offset = match verified_header_offset {
                            Some(hdr) => hdr,
                            None => return Ok(None),
                        };

                        return Ok(Some(ZipMetadataInfo {
                            entry_name: "payload.bin".to_string(),
                            header_offset,
                            payload_data_offset: offset,
                            uncompressed_size: size,
                            compressed_size: size,
                            compression_method: 0,
                            total_entries: None,
                            central_directory_offset: None,
                            archive_size: file_size,
                        }));
                    }
                }
                return Ok(None);
            }

            // If bit 3 (data descriptor) or ZIP64 indicator (0xFFFFFFFF) is set on non-metadata entry,
            // we cannot safely calculate next_pos from local header alone; bail out to Central Directory scan.
            if (flags & 0x0008) != 0
                || compressed_size == 0xFFFF_FFFF
                || uncompressed_size == 0xFFFF_FFFF
            {
                break;
            }

            let next_pos = match pos
                .checked_add(30)
                .and_then(|p| p.checked_add(filename_len))
                .and_then(|p| p.checked_add(extra_len))
                .and_then(|p| p.checked_add(compressed_size))
            {
                Some(p) if p > pos => p,
                _ => break,
            };
            pos = next_pos;
        }

        Ok(None)
    }

    /// populate central directory information (total_entries and central_directory_offset)
    /// by reading the EOCD record.
    pub async fn populate_central_directory_info<I: ZipIO>(
        io: &I,
        info: &mut ZipMetadataInfo,
    ) -> Result<()> {
        if info.total_entries.is_none() || info.central_directory_offset.is_none() {
            let (cd_offset, num_entries) = Self::get_central_directory_info(io).await?;
            info.central_directory_offset = Some(cd_offset);
            info.total_entries = Some(num_entries);
        }
        Ok(())
    }

    /// get full zip metadata information by always reading the central directory
    pub async fn get_full_zip_info<I: ZipIO>(io: &I) -> Result<ZipMetadataInfo> {
        let archive_size = io.size().await?;
        let (cd_offset, num_entries) = Self::get_central_directory_info(io).await?;
        let entry = Self::find_payload_entry(io).await?;
        let payload_data_offset = Self::get_data_offset(io, &entry).await?;
        Self::verify_payload_magic(io, payload_data_offset).await?;

        Ok(ZipMetadataInfo {
            entry_name: entry.name,
            header_offset: entry.offset,
            payload_data_offset,
            uncompressed_size: entry.uncompressed_size,
            compressed_size: entry.compressed_size,
            compression_method: entry.compression_method,
            total_entries: Some(num_entries),
            central_directory_offset: Some(cd_offset),
            archive_size,
        })
    }

    /// get zip metadata information for payload.bin, trying the fast OTA metadata path first
    pub async fn get_zip_info<I: ZipIO>(io: &I) -> Result<ZipMetadataInfo> {
        // Fast path: try Android OTA streaming metadata at the start of the archive
        if let Ok(Some(fast_info)) = Self::try_fast_ota_metadata(io).await {
            return Ok(fast_info);
        }

        Self::get_full_zip_info(io).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemoryZipIO {
        data: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl ZipIO for MemoryZipIO {
        async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
            let offset = offset as usize;
            if offset
                .checked_add(buf.len())
                .map(|end| end <= self.data.len())
                != Some(true)
            {
                return Err(anyhow!("Read out of bounds"));
            }
            buf.copy_from_slice(&self.data[offset..offset + buf.len()]);
            Ok(())
        }

        async fn size(&self) -> Result<u64> {
            Ok(self.data.len() as u64)
        }
    }

    struct SyntheticEntryConfig {
        name: String,
        flags: u16,
        compression: u16,
        compressed_size: u32,
        uncompressed_size: u32,
        extra: Vec<u8>,
        data: Vec<u8>,
    }

    struct SyntheticExpectedMeta {
        payload_header_offset: u64,
        payload_data_offset: u64,
        payload_size: u64,
        total_entries: usize,
        central_directory_offset: u64,
    }

    struct SyntheticZipBuilder {
        preceding_entry: Option<SyntheticEntryConfig>,
        include_metadata: bool,
        metadata_name: String,
        metadata_flags: u16,
        metadata_compression: u16,
        metadata_compressed_size: Option<u32>,
        metadata_uncompressed_size: Option<u32>,
        metadata_extra: Vec<u8>,
        meta_payload_name: String,
        meta_payload_offset_override: Option<u64>,
        meta_payload_size_override: Option<u64>,

        payload_name: String,
        payload_flags: u16,
        payload_compression: u16,
        payload_compressed_size: Option<u32>,
        payload_uncompressed_size: Option<u32>,
        payload_extra: Vec<u8>,
        payload_data: Vec<u8>,

        truncate_to: Option<usize>,
    }

    impl SyntheticZipBuilder {
        fn new() -> Self {
            let mut payload_data = b"CrAU".to_vec();
            payload_data.extend_from_slice(&[0u8; 12]);

            Self {
                preceding_entry: None,
                include_metadata: true,
                metadata_name: "META-INF/com/android/metadata".to_string(),
                metadata_flags: 0,
                metadata_compression: 0,
                metadata_compressed_size: None,
                metadata_uncompressed_size: None,
                metadata_extra: Vec::new(),
                meta_payload_name: "payload.bin".to_string(),
                meta_payload_offset_override: None,
                meta_payload_size_override: None,

                payload_name: "payload.bin".to_string(),
                payload_flags: 0,
                payload_compression: 0,
                payload_compressed_size: None,
                payload_uncompressed_size: None,
                payload_extra: Vec::new(),
                payload_data,

                truncate_to: None,
            }
        }

        fn build(self) -> (Vec<u8>, SyntheticExpectedMeta) {
            let mut data = Vec::new();
            let mut cd_entries = Vec::new();

            // 1. Preceding entry if any
            if let Some(entry) = self.preceding_entry {
                let local_header_offset = data.len() as u32;
                let name_bytes = entry.name.as_bytes();

                data.extend_from_slice(&LOCAL_FILE_HEADER_SIGNATURE);
                data.extend_from_slice(&20u16.to_le_bytes()); // version
                data.extend_from_slice(&entry.flags.to_le_bytes());
                data.extend_from_slice(&entry.compression.to_le_bytes());
                data.extend_from_slice(&[0u8; 4]); // time/date
                data.extend_from_slice(&[0u8; 4]); // crc32
                data.extend_from_slice(&entry.compressed_size.to_le_bytes());
                data.extend_from_slice(&entry.uncompressed_size.to_le_bytes());
                data.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                data.extend_from_slice(&(entry.extra.len() as u16).to_le_bytes());
                data.extend_from_slice(name_bytes);
                data.extend_from_slice(&entry.extra);
                data.extend_from_slice(&entry.data);

                cd_entries.push((
                    entry.name,
                    entry.flags,
                    entry.compression,
                    entry.compressed_size,
                    entry.uncompressed_size,
                    entry.extra,
                    local_header_offset,
                ));
            }

            // Calculate payload layout
            let payload_size = self.payload_data.len() as u64;
            let (payload_header_offset, payload_data_offset) = if self.include_metadata {
                let meta_header_offset = data.len();
                let meta_name_bytes = self.metadata_name.as_bytes();
                let meta_header_len = 30 + meta_name_bytes.len() + self.metadata_extra.len();
                let meta_content_len = 121usize; // fixed formatted length
                let p_hdr = meta_header_offset + meta_header_len + meta_content_len;
                let p_data = p_hdr + 30 + self.payload_name.len() + self.payload_extra.len();
                (p_hdr, p_data)
            } else {
                let p_hdr = data.len();
                let p_data = p_hdr + 30 + self.payload_name.len() + self.payload_extra.len();
                (p_hdr, p_data)
            };

            // 2. Metadata entry if included
            if self.include_metadata {
                let meta_header_offset = data.len() as u32;
                let meta_name_bytes = self.metadata_name.as_bytes();
                let meta_offset = self
                    .meta_payload_offset_override
                    .unwrap_or(payload_data_offset as u64);
                let meta_size = self.meta_payload_size_override.unwrap_or(payload_size);

                let line = format!(
                    "ota-streaming-property-files={}:{}:{}\n",
                    self.meta_payload_name, meta_offset, meta_size
                );
                let mut meta_content = line.into_bytes();
                if meta_content.len() < 120 {
                    meta_content.resize(120, b' ');
                    meta_content.push(b'\n');
                }
                assert_eq!(meta_content.len(), 121);

                let comp_size = self
                    .metadata_compressed_size
                    .unwrap_or(meta_content.len() as u32);
                let uncomp_size = self
                    .metadata_uncompressed_size
                    .unwrap_or(meta_content.len() as u32);

                data.extend_from_slice(&LOCAL_FILE_HEADER_SIGNATURE);
                data.extend_from_slice(&20u16.to_le_bytes());
                data.extend_from_slice(&self.metadata_flags.to_le_bytes());
                data.extend_from_slice(&self.metadata_compression.to_le_bytes());
                data.extend_from_slice(&[0u8; 4]);
                data.extend_from_slice(&[0u8; 4]);
                data.extend_from_slice(&comp_size.to_le_bytes());
                data.extend_from_slice(&uncomp_size.to_le_bytes());
                data.extend_from_slice(&(meta_name_bytes.len() as u16).to_le_bytes());
                data.extend_from_slice(&(self.metadata_extra.len() as u16).to_le_bytes());
                data.extend_from_slice(meta_name_bytes);
                data.extend_from_slice(&self.metadata_extra);
                data.extend_from_slice(&meta_content);

                cd_entries.push((
                    self.metadata_name,
                    self.metadata_flags,
                    self.metadata_compression,
                    comp_size,
                    uncomp_size,
                    self.metadata_extra,
                    meta_header_offset,
                ));
            }

            // 3. Payload entry
            let p_hdr_off = data.len() as u32;
            assert_eq!(p_hdr_off as usize, payload_header_offset);
            let p_name_bytes = self.payload_name.as_bytes();
            let p_comp_size = self
                .payload_compressed_size
                .unwrap_or(self.payload_data.len() as u32);
            let p_uncomp_size = self
                .payload_uncompressed_size
                .unwrap_or(self.payload_data.len() as u32);

            data.extend_from_slice(&LOCAL_FILE_HEADER_SIGNATURE);
            data.extend_from_slice(&20u16.to_le_bytes());
            data.extend_from_slice(&self.payload_flags.to_le_bytes());
            data.extend_from_slice(&self.payload_compression.to_le_bytes());
            data.extend_from_slice(&[0u8; 4]);
            data.extend_from_slice(&[0u8; 4]);
            data.extend_from_slice(&p_comp_size.to_le_bytes());
            data.extend_from_slice(&p_uncomp_size.to_le_bytes());
            data.extend_from_slice(&(p_name_bytes.len() as u16).to_le_bytes());
            data.extend_from_slice(&(self.payload_extra.len() as u16).to_le_bytes());
            data.extend_from_slice(p_name_bytes);
            data.extend_from_slice(&self.payload_extra);
            data.extend_from_slice(&self.payload_data);

            let actual_p_data = data.len() - self.payload_data.len();
            assert_eq!(actual_p_data, payload_data_offset);

            cd_entries.push((
                self.payload_name,
                self.payload_flags,
                self.payload_compression,
                p_comp_size,
                p_uncomp_size,
                self.payload_extra,
                p_hdr_off,
            ));

            // 4. Central Directory
            let cd_offset = data.len() as u32;
            for (name, flags, comp, c_size, u_size, extra, lfh_off) in &cd_entries {
                let nb = name.as_bytes();
                data.extend_from_slice(&CENTRAL_DIR_HEADER_SIGNATURE);
                data.extend_from_slice(&20u16.to_le_bytes()); // version made by
                data.extend_from_slice(&20u16.to_le_bytes()); // version needed
                data.extend_from_slice(&flags.to_le_bytes());
                data.extend_from_slice(&comp.to_le_bytes());
                data.extend_from_slice(&[0u8; 4]); // time/date
                data.extend_from_slice(&[0u8; 4]); // crc32
                data.extend_from_slice(&c_size.to_le_bytes());
                data.extend_from_slice(&u_size.to_le_bytes());
                data.extend_from_slice(&(nb.len() as u16).to_le_bytes());
                data.extend_from_slice(&(extra.len() as u16).to_le_bytes());
                data.extend_from_slice(&[0u8; 2]); // comment len
                data.extend_from_slice(&[0u8; 2]); // disk number start
                data.extend_from_slice(&[0u8; 2]); // internal attrs
                data.extend_from_slice(&[0u8; 4]); // external attrs
                data.extend_from_slice(&lfh_off.to_le_bytes());
                data.extend_from_slice(nb);
                data.extend_from_slice(extra);
            }
            let cd_size = (data.len() - cd_offset as usize) as u32;
            let num_entries = cd_entries.len();

            // 5. EOCD
            data.extend_from_slice(&EOCD_SIGNATURE);
            data.extend_from_slice(&[0u8; 4]); // disk numbers
            data.extend_from_slice(&(num_entries as u16).to_le_bytes());
            data.extend_from_slice(&(num_entries as u16).to_le_bytes());
            data.extend_from_slice(&cd_size.to_le_bytes());
            data.extend_from_slice(&cd_offset.to_le_bytes());
            data.extend_from_slice(&[0u8; 2]); // comment len

            if let Some(len) = self.truncate_to {
                data.truncate(len);
            }

            let expected_meta = SyntheticExpectedMeta {
                payload_header_offset: payload_header_offset as u64,
                payload_data_offset: payload_data_offset as u64,
                payload_size,
                total_entries: num_entries,
                central_directory_offset: cd_offset as u64,
            };

            (data, expected_meta)
        }
    }

    #[tokio::test]
    async fn test_1_valid_ota_zip() {
        let builder = SyntheticZipBuilder::new();
        let (data, expected) = builder.build();
        let io = MemoryZipIO { data };

        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_some());
        let info = res.unwrap();
        assert_eq!(info.entry_name, "payload.bin");
        assert_eq!(info.payload_data_offset, expected.payload_data_offset);
        assert_eq!(info.header_offset, expected.payload_header_offset);
        assert_eq!(info.uncompressed_size, expected.payload_size);
        assert_eq!(info.compressed_size, expected.payload_size);
        assert_eq!(info.compression_method, 0);
        assert_eq!(info.total_entries, None);
        assert_eq!(info.central_directory_offset, None);
    }

    #[tokio::test]
    async fn test_2_metadata_offset_not_crau_fallback() {
        let mut builder = SyntheticZipBuilder::new();
        // Point metadata offset to offset 0 (starts with PK\x03\x04, not CrAU)
        builder.meta_payload_offset_override = Some(0);
        let (data, expected) = builder.build();
        let io = MemoryZipIO { data };

        let fast_res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(fast_res.is_none());

        let full_res = ZipParser::get_zip_info(&io).await.unwrap();
        assert_eq!(full_res.payload_data_offset, expected.payload_data_offset);
        assert_eq!(full_res.total_entries, Some(expected.total_entries));
        assert_eq!(
            full_res.central_directory_offset,
            Some(expected.central_directory_offset)
        );
    }

    #[tokio::test]
    async fn test_3_no_metadata_entry_fallback() {
        let mut builder = SyntheticZipBuilder::new();
        builder.include_metadata = false;
        let (data, expected) = builder.build();
        let io = MemoryZipIO { data };

        let fast_res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(fast_res.is_none());

        let full_res = ZipParser::get_zip_info(&io).await.unwrap();
        assert_eq!(full_res.payload_data_offset, expected.payload_data_offset);
        assert_eq!(full_res.total_entries, Some(expected.total_entries));
        assert_eq!(
            full_res.central_directory_offset,
            Some(expected.central_directory_offset)
        );
    }

    #[tokio::test]
    async fn test_4_preceding_entry_flag_bit_3() {
        let mut builder = SyntheticZipBuilder::new();
        builder.preceding_entry = Some(SyntheticEntryConfig {
            name: "dummy.txt".to_string(),
            flags: 0x0008, // bit 3 set (data descriptor)
            compression: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            extra: Vec::new(),
            data: b"dummy data".to_vec(),
        });
        let (data, _) = builder.build();
        let io = MemoryZipIO { data };

        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_5_preceding_entry_zip64() {
        let mut builder = SyntheticZipBuilder::new();
        builder.preceding_entry = Some(SyntheticEntryConfig {
            name: "dummy.txt".to_string(),
            flags: 0,
            compression: 0,
            compressed_size: 0xFFFF_FFFF,
            uncompressed_size: 0xFFFF_FFFF,
            extra: Vec::new(),
            data: b"dummy data".to_vec(),
        });
        let (data, _) = builder.build();
        let io = MemoryZipIO { data };

        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_6_metadata_uncompressed_size_exceeds_1mb() {
        let mut builder = SyntheticZipBuilder::new();
        builder.metadata_uncompressed_size = Some(2 * 1024 * 1024); // 2 MB
        let (data, _) = builder.build();
        let io = MemoryZipIO { data };

        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_7_metadata_offset_size_overflow_or_exceeds_filesize() {
        // 7a: offset + size overflows u64
        let mut builder = SyntheticZipBuilder::new();
        builder.meta_payload_offset_override = Some(u64::MAX - 5);
        builder.meta_payload_size_override = Some(100);
        let (data, _) = builder.build();
        let io = MemoryZipIO { data };
        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());

        // 7b: offset + size exceeds file size
        let mut builder = SyntheticZipBuilder::new();
        builder.meta_payload_size_override = Some(1_000_000);
        let (data, _) = builder.build();
        let io = MemoryZipIO { data };
        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_8_payload_local_header_size_disagrees() {
        let mut builder = SyntheticZipBuilder::new();
        builder.payload_uncompressed_size = Some(999);
        let (data, _) = builder.build();
        let io = MemoryZipIO { data };

        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_9_payload_local_header_not_stored() {
        let mut builder = SyntheticZipBuilder::new();
        builder.payload_compression = 8; // Deflated
        let (data, _) = builder.build();
        let io = MemoryZipIO { data };

        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_10_payload_name_not_exact_match() {
        let mut builder = SyntheticZipBuilder::new();
        builder.meta_payload_name = "x/payload.bin".to_string();
        let (data, _) = builder.build();
        let io = MemoryZipIO { data };

        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_11_file_smaller_than_128_bytes() {
        let mut builder = SyntheticZipBuilder::new();
        builder.truncate_to = Some(100);
        let (data, _) = builder.build();
        assert!(data.len() < 128);
        let io = MemoryZipIO { data };

        let res = ZipParser::try_fast_ota_metadata(&io).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_12_get_full_zip_info_matching_layout() {
        let builder = SyntheticZipBuilder::new();
        let (data, expected) = builder.build();
        let io = MemoryZipIO { data };

        let fast_info = ZipParser::try_fast_ota_metadata(&io)
            .await
            .unwrap()
            .unwrap();
        let full_info = ZipParser::get_full_zip_info(&io).await.unwrap();

        assert_eq!(full_info.payload_data_offset, fast_info.payload_data_offset);
        assert_eq!(full_info.header_offset, fast_info.header_offset);
        assert_eq!(full_info.uncompressed_size, fast_info.uncompressed_size);
        assert_eq!(full_info.total_entries, Some(expected.total_entries));
        assert_eq!(
            full_info.central_directory_offset,
            Some(expected.central_directory_offset)
        );
    }
}
