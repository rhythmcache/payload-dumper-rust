// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::payload::payload_dumper::{PayloadReader, ProgressReporter};
use crate::structs::{Extent, InstallOperation, install_operation};

const MAX_OPERATION_SIZE: usize = 512 * 1024 * 1024; // 512 MB safety limit

pub struct DiffContext {
    pub source_dir: PathBuf,
    pub block_size: u64,
}

impl DiffContext {
    pub fn new(source_dir: PathBuf, block_size: u64) -> Self {
        Self {
            source_dir,
            block_size,
        }
    }
    async fn read_source_extents(
        &self,
        source_file: &mut File,
        extents: &[Extent],
    ) -> Result<Vec<u8>> {
        let total_size: u64 = extents
            .iter()
            .map(|e| e.num_blocks.unwrap_or(0) * self.block_size)
            .sum();

        if total_size > MAX_OPERATION_SIZE as u64 {
            return Err(anyhow!(
                "Source extents total size {} exceeds safety limit",
                total_size
            ));
        }

        let mut data = Vec::with_capacity(total_size as usize);

        for (i, extent) in extents.iter().enumerate() {
            let start_block = extent.start_block.unwrap_or(0);
            let num_blocks = extent.num_blocks.unwrap_or(0);

            if num_blocks == 0 {
                continue;
            }

            let offset = start_block
                .checked_mul(self.block_size)
                .ok_or_else(|| anyhow!("Offset overflow in extent {}", i))?;

            let length = num_blocks
                .checked_mul(self.block_size)
                .ok_or_else(|| anyhow!("Length overflow in extent {}", i))?;

            source_file
                .seek(std::io::SeekFrom::Start(offset))
                .await
                .context(format!("Failed to seek to extent {} offset {}", i, offset))?;

            let current_len = data.len();
            data.resize(current_len + length as usize, 0);

            source_file
                .read_exact(&mut data[current_len..])
                .await
                .context(format!("Failed to read extent {} ({} bytes)", i, length))?;
        }

        Ok(data)
    }

    async fn write_dst_extents(
        &self,
        out_file: &mut File,
        extents: &[Extent],
        data: &[u8],
    ) -> Result<()> {
        let expected_size: u64 = extents
            .iter()
            .map(|e| e.num_blocks.unwrap_or(0) * self.block_size)
            .sum();

        if data.len() != expected_size as usize {
            return Err(anyhow!(
                "Data size mismatch: expected {} bytes, got {} bytes",
                expected_size,
                data.len()
            ));
        }

        let mut data_offset = 0usize;

        for (i, extent) in extents.iter().enumerate() {
            let start_block = extent.start_block.unwrap_or(0);
            let num_blocks = extent.num_blocks.unwrap_or(0);

            if num_blocks == 0 {
                continue;
            }

            let offset = start_block
                .checked_mul(self.block_size)
                .ok_or_else(|| anyhow!("Offset overflow in dst extent {}", i))?;

            let length = (num_blocks * self.block_size) as usize;

            if data_offset + length > data.len() {
                return Err(anyhow!(
                    "Insufficient data for extent {}: need {} bytes, have {} remaining",
                    i,
                    length,
                    data.len() - data_offset
                ));
            }

            out_file
                .seek(std::io::SeekFrom::Start(offset))
                .await
                .context(format!(
                    "Failed to seek to dst extent {} offset {}",
                    i, offset
                ))?;

            out_file
                .write_all(&data[data_offset..data_offset + length])
                .await
                .context(format!("Failed to write extent {} ({} bytes)", i, length))?;

            data_offset += length;
        }

        Ok(())
    }
}

#[cfg(feature = "diff_ota")]
pub async fn process_diff_operation(
    operation_index: usize,
    op: &InstallOperation,
    ctx: &DiffContext,
    partition_name: &str,
    source_file: &mut File,
    out_file: &mut File,
    payload_reader: &mut dyn PayloadReader,
    data_offset: u64,
    reporter: &dyn ProgressReporter,
) -> Result<()> {
    match op.r#type() {
        install_operation::Type::SourceCopy => {
            let source_data = ctx
                .read_source_extents(source_file, &op.src_extents)
                .await
                .context("Failed to read source extents for SOURCE_COPY")?;

            ctx.write_dst_extents(out_file, &op.dst_extents, &source_data)
                .await
                .context("Failed to write destination extents for SOURCE_COPY")?;
        }

        install_operation::Type::SourceBsdiff => {
            let source_data = ctx
                .read_source_extents(source_file, &op.src_extents)
                .await
                .context("Failed to read source extents for SOURCE_BSDIFF")?;

            let patch_offset = data_offset + op.data_offset.unwrap_or(0);
            let patch_length = op.data_length.unwrap_or(0);

            if patch_length > MAX_OPERATION_SIZE as u64 {
                return Err(anyhow!("Patch size {} exceeds safety limit", patch_length));
            }

            let mut patch_stream = payload_reader
                .read_range(patch_offset, patch_length)
                .await
                .context("Failed to read patch data")?;

            let mut patch_data = Vec::with_capacity(patch_length as usize);
            patch_stream
                .read_to_end(&mut patch_data)
                .await
                .context("Failed to read patch stream")?;

            let mut patched_data = Vec::new();
            bsdiff_android::patch_bsdf2(&source_data, &patch_data, &mut patched_data)
                .map_err(|e| anyhow!("BSDF2 patch failed: {}", e))?;

            let expected_size: u64 = op
                .dst_extents
                .iter()
                .map(|e| e.num_blocks.unwrap_or(0) * ctx.block_size)
                .sum();

            if patched_data.len() != expected_size as usize {
                return Err(anyhow!(
                    "Patched data size mismatch: expected {} bytes, got {} bytes",
                    expected_size,
                    patched_data.len()
                ));
            }

            ctx.write_dst_extents(out_file, &op.dst_extents, &patched_data)
                .await
                .context("Failed to write patched data")?;
        }

        install_operation::Type::BrotliBsdiff => {
            let source_data = ctx
                .read_source_extents(source_file, &op.src_extents)
                .await
                .context("Failed to read source extents for BROTLI_BSDIFF")?;

            let patch_offset = data_offset + op.data_offset.unwrap_or(0);
            let patch_length = op.data_length.unwrap_or(0);

            if patch_length > MAX_OPERATION_SIZE as u64 {
                return Err(anyhow!(
                    "Compressed patch size {} exceeds safety limit",
                    patch_length
                ));
            }

            let mut patch_stream = payload_reader
                .read_range(patch_offset, patch_length)
                .await
                .context("Failed to read compressed patch data")?;

            let mut compressed_data = Vec::with_capacity(patch_length as usize);
            patch_stream
                .read_to_end(&mut compressed_data)
                .await
                .context("Failed to read compressed stream")?;

            let mut patch_data = Vec::new();
            let mut decompressor = brotli::Decompressor::new(&compressed_data[..], 4096);
            std::io::Read::read_to_end(&mut decompressor, &mut patch_data)
                .map_err(|e| anyhow!("Failed to decompress Brotli patch: {}", e))?;

            if patch_data.len() > MAX_OPERATION_SIZE {
                return Err(anyhow!(
                    "Decompressed patch size {} exceeds safety limit",
                    patch_data.len()
                ));
            }

            let mut patched_data = Vec::new();
            bsdiff_android::patch_bsdf2(&source_data, &patch_data, &mut patched_data)
                .map_err(|e| anyhow!("Brotli-BSDF2 patch failed: {}", e))?;

            let expected_size: u64 = op
                .dst_extents
                .iter()
                .map(|e| e.num_blocks.unwrap_or(0) * ctx.block_size)
                .sum();

            if patched_data.len() != expected_size as usize {
                return Err(anyhow!(
                    "Patched data size mismatch: expected {} bytes, got {} bytes",
                    expected_size,
                    patched_data.len()
                ));
            }

            ctx.write_dst_extents(out_file, &op.dst_extents, &patched_data)
                .await
                .context("Failed to write Brotli-patched data")?;
        }

        install_operation::Type::Lz4diffBsdiff => {
            let source_data = ctx
                .read_source_extents(source_file, &op.src_extents)
                .await
                .context("Failed to read source extents for LZ4DIFF_BSDIFF")?;

            let patch_offset = data_offset + op.data_offset.unwrap_or(0);
            let patch_length = op.data_length.unwrap_or(0);

            if patch_length > MAX_OPERATION_SIZE as u64 {
                return Err(anyhow!(
                    "Compressed patch size {} exceeds safety limit",
                    patch_length
                ));
            }

            let mut patch_stream = payload_reader
                .read_range(patch_offset, patch_length)
                .await
                .context("Failed to read LZ4 compressed patch data")?;

            let mut compressed_data = Vec::with_capacity(patch_length as usize);
            patch_stream
                .read_to_end(&mut compressed_data)
                .await
                .context("Failed to read compressed stream")?;

            let patch_data = match lz4_flex::decompress_size_prepended(&compressed_data) {
                Ok(data) => data,
                Err(_) => lz4_flex::block::decompress(&compressed_data, MAX_OPERATION_SIZE)
                    .map_err(|e| anyhow!("Failed to decompress LZ4 patch: {}", e))?,
            };

            if patch_data.len() > MAX_OPERATION_SIZE {
                return Err(anyhow!(
                    "Decompressed patch size {} exceeds safety limit",
                    patch_data.len()
                ));
            }

            // Apply BSDF2 patch
            let mut patched_data = Vec::new();
            bsdiff_android::patch_bsdf2(&source_data, &patch_data, &mut patched_data)
                .map_err(|e| anyhow!("LZ4-BSDF2 patch failed: {}", e))?;

            // Validate output size
            let expected_size: u64 = op
                .dst_extents
                .iter()
                .map(|e| e.num_blocks.unwrap_or(0) * ctx.block_size)
                .sum();

            if patched_data.len() != expected_size as usize {
                return Err(anyhow!(
                    "Patched data size mismatch: expected {} bytes, got {} bytes",
                    expected_size,
                    patched_data.len()
                ));
            }

            // Write to destination
            ctx.write_dst_extents(out_file, &op.dst_extents, &patched_data)
                .await
                .context("Failed to write LZ4-patched data")?;
        }

        install_operation::Type::Puffdiff | install_operation::Type::Lz4diffPuffdiff => {
            reporter.on_warning(
                partition_name,
                operation_index,
                "PUFFDIFF operations not supported yet".to_string(),
            );
            return Err(anyhow!("PUFFDIFF operation not supported"));
        }

        install_operation::Type::Zucchini => {
            reporter.on_warning(
                partition_name,
                operation_index,
                "ZUCCHINI operations not supported yet".to_string(),
            );
            return Err(anyhow!("ZUCCHINI operation not supported"));
        }

        _ => {
            return Err(anyhow!(
                "Operation type {:?} is not a differential operation",
                op.r#type()
            ));
        }
    }

    Ok(())
}
