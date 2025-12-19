// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache

use anyhow::{Result, anyhow};
use async_compression::tokio::bufread::{BrotliDecoder, Lz4Decoder};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};

use crate::payload::payload_dumper::{PayloadReader, ProgressReporter};
use crate::structs::{Extent, InstallOperation, install_operation};

const BUFREADER_SIZE: usize = 64 * 1024;

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
        let mut data = Vec::new();
        for extent in extents {
            let start_block = extent.start_block.unwrap_or(0);
            let num_blocks = extent.num_blocks.unwrap_or(0);
            let offset = start_block * self.block_size;
            let length = num_blocks * self.block_size;

            source_file.seek(std::io::SeekFrom::Start(offset)).await?;
            let mut extent_data = vec![0u8; length as usize];
            source_file.read_exact(&mut extent_data).await?;
            data.extend_from_slice(&extent_data);
        }
        Ok(data)
    }

    async fn write_dst_extents(
        &self,
        out_file: &mut File,
        extents: &[Extent],
        data: &[u8],
    ) -> Result<()> {
        let mut data_offset = 0;
        for extent in extents {
            let start_block = extent.start_block.unwrap_or(0);
            let num_blocks = extent.num_blocks.unwrap_or(0);
            let offset = start_block * self.block_size;
            let length = (num_blocks * self.block_size) as usize;

            out_file.seek(std::io::SeekFrom::Start(offset)).await?;
            out_file
                .write_all(&data[data_offset..data_offset + length])
                .await?;
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
                .await?;
            ctx.write_dst_extents(out_file, &op.dst_extents, &source_data)
                .await?;
        }
        install_operation::Type::SourceBsdiff => {
            let source_data = ctx
                .read_source_extents(source_file, &op.src_extents)
                .await?;
            let patch_offset = data_offset + op.data_offset.unwrap_or(0);
            let patch_length = op.data_length.unwrap_or(0);

            let mut patch_stream = payload_reader
                .read_range(patch_offset, patch_length)
                .await?;
            let mut patch_data = Vec::new();
            patch_stream.read_to_end(&mut patch_data).await?;

            let mut patched_data = Vec::new();
            bsdiff::patch(&source_data, &mut patch_data.as_slice(), &mut patched_data)
                .map_err(|e| anyhow!("bsdiff patch failed: {}", e))?;

            ctx.write_dst_extents(out_file, &op.dst_extents, &patched_data)
                .await?;
        }
        install_operation::Type::BrotliBsdiff => {
            let source_data = ctx
                .read_source_extents(source_file, &op.src_extents)
                .await?;
            let patch_offset = data_offset + op.data_offset.unwrap_or(0);
            let patch_length = op.data_length.unwrap_or(0);

            let patch_stream = payload_reader
                .read_range(patch_offset, patch_length)
                .await?;
            let mut decoder =
                BrotliDecoder::new(BufReader::with_capacity(BUFREADER_SIZE, patch_stream));
            let mut patch_data = Vec::new();
            decoder.read_to_end(&mut patch_data).await?;

            let mut patched_data = Vec::new();
            bsdiff::patch(&source_data, &mut patch_data.as_slice(), &mut patched_data)
                .map_err(|e| anyhow!("brotli-bsdiff patch failed: {}", e))?;

            ctx.write_dst_extents(out_file, &op.dst_extents, &patched_data)
                .await?;
        }
        install_operation::Type::Lz4diffBsdiff => {
            let source_data = ctx
                .read_source_extents(source_file, &op.src_extents)
                .await?;
            let patch_offset = data_offset + op.data_offset.unwrap_or(0);
            let patch_length = op.data_length.unwrap_or(0);

            let patch_stream = payload_reader
                .read_range(patch_offset, patch_length)
                .await?;
            let mut decoder =
                Lz4Decoder::new(BufReader::with_capacity(BUFREADER_SIZE, patch_stream));
            let mut patch_data = Vec::new();
            decoder.read_to_end(&mut patch_data).await?;

            let mut patched_data = Vec::new();
            bsdiff::patch(&source_data, &mut patch_data.as_slice(), &mut patched_data)
                .map_err(|e| anyhow!("lz4diff-bsdiff patch failed: {}", e))?;

            ctx.write_dst_extents(out_file, &op.dst_extents, &patched_data)
                .await?;
        }
        install_operation::Type::Puffdiff | install_operation::Type::Lz4diffPuffdiff => {
            reporter.on_warning(
                partition_name,
                operation_index,
                "PUFFDIFF not supported yet".to_string(),
            );
            return Err(anyhow!("PUFFDIFF operation not supported"));
        }
        install_operation::Type::Zucchini => {
            reporter.on_warning(
                partition_name,
                operation_index,
                "ZUCCHINI not supported yet".to_string(),
            );
            return Err(anyhow!("ZUCCHINI operation not supported"));
        }
        _ => return Err(anyhow!("Not a differential operation")),
    }
    Ok(())
}
