// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

#![allow(dead_code)]
use anyhow::{Result, anyhow};
use async_compression::tokio::bufread::{BzDecoder, XzDecoder, ZstdDecoder};
use async_trait::async_trait;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};

pub use crate::structs::PartitionUpdate;
use crate::structs::{InstallOperation, install_operation};

#[cfg(feature = "diff_ota")]
use crate::payload::diff::{DiffContext, DiffOperationParams, process_diff_operation};
use crate::utils::is_diff_operation;

const BUFREADER_SIZE: usize = 64 * 1024; // 64 KB for BufReader (decompression streams)
const COPY_BUFFER_SIZE: usize = 128 * 1024; // 128 KB for direct copy operations

/// progress reporting trait for partition extraction
/// implement this to receive progress updates during extraction
#[async_trait]
pub trait ProgressReporter: Send + Sync {
    /// called when extraction starts for a partition
    fn on_start(&self, partition_name: &str, total_operations: u64);

    /// called after each operation completes
    fn on_progress(&self, partition_name: &str, current_op: u64, total_ops: u64);

    /// called when extraction completes successfully
    fn on_complete(&self, partition_name: &str, total_operations: u64);

    /// called when a non-fatal warning occurs (operation skipped, etc.)
    fn on_warning(&self, partition_name: &str, operation_index: usize, message: String);

    /// check if cancellation has been requested
    /// return true if extraction should be cancelled
    fn is_cancelled(&self) -> bool {
        false // default implementation for backwards compatibility
    }
}

/// no-op reporter for headless/library use
pub struct NoOpReporter;

impl ProgressReporter for NoOpReporter {
    fn on_start(&self, _: &str, _: u64) {}
    fn on_progress(&self, _: &str, _: u64, _: u64) {}
    fn on_complete(&self, _: &str, _: u64) {}
    fn on_warning(&self, _: &str, _: usize, _: String) {}
}

#[async_trait]
pub trait AsyncPayloadRead: Send + Sync {
    async fn open_reader(&self) -> Result<Box<dyn PayloadReader>>;
}

#[async_trait]
pub trait PayloadReader: Send {
    async fn read_range(
        &mut self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send + '_>>>;
}

#[async_trait]
impl<T: AsyncPayloadRead> AsyncPayloadRead for Arc<T> {
    async fn open_reader(&self) -> Result<Box<dyn PayloadReader>> {
        (**self).open_reader().await
    }
}

#[async_trait]
impl AsyncPayloadRead for Arc<dyn AsyncPayloadRead> {
    async fn open_reader(&self) -> Result<Box<dyn PayloadReader>> {
        (**self).open_reader().await
    }
}

/// custom copy function with reusable buffer
async fn copy_with_buffer<R, W>(reader: &mut R, writer: &mut W, buf: &mut [u8]) -> Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut total = 0u64;

    loop {
        let n = reader.read(buf).await?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await?;
        total += n as u64;
    }

    Ok(total)
}

/// context for processing operations -> groups related parameters
struct OperationContext<'a> {
    data_offset: u64,
    block_size: u64,
    payload_reader: &'a mut dyn PayloadReader,
    out_file: &'a mut File,
    copy_buffer: &'a mut [u8],
    zero_buffer: &'a [u8],
    #[cfg(feature = "diff_ota")]
    diff_ctx: Option<&'a DiffContext>,
    #[cfg(feature = "diff_ota")]
    source_file: Option<&'a mut File>,
}

async fn process_operation_streaming(
    operation_index: usize,
    op: &InstallOperation,
    ctx: &mut OperationContext<'_>,
    reporter: &dyn ProgressReporter,
    partition_name: &str,
) -> Result<()> {
    let offset = ctx.data_offset + op.data_offset.unwrap_or(0);
    let length = op.data_length.unwrap_or(0);

    match op.r#type() {
        install_operation::Type::Replace => {
            let mut stream = ctx.payload_reader.read_range(offset, length).await?;
            ctx.out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * ctx.block_size,
                ))
                .await?;
            copy_with_buffer(&mut stream, ctx.out_file, ctx.copy_buffer).await?;
        }
        install_operation::Type::ReplaceXz => {
            let stream = ctx.payload_reader.read_range(offset, length).await?;
            let mut decoder = XzDecoder::new(BufReader::with_capacity(BUFREADER_SIZE, stream));
            ctx.out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * ctx.block_size,
                ))
                .await?;
            match copy_with_buffer(&mut decoder, ctx.out_file, ctx.copy_buffer).await {
                Ok(_) => {}
                Err(e) => {
                    reporter.on_warning(
                        partition_name,
                        operation_index,
                        format!("XZ decompression error: {}", e),
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::ReplaceBz => {
            let stream = ctx.payload_reader.read_range(offset, length).await?;
            let mut decoder = BzDecoder::new(BufReader::with_capacity(BUFREADER_SIZE, stream));
            ctx.out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * ctx.block_size,
                ))
                .await?;
            match copy_with_buffer(&mut decoder, ctx.out_file, ctx.copy_buffer).await {
                Ok(_) => {}
                Err(e) => {
                    reporter.on_warning(
                        partition_name,
                        operation_index,
                        format!("BZ2 decompression error: {}", e),
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Zstd => {
            let stream = ctx.payload_reader.read_range(offset, length).await?;
            let mut decoder = ZstdDecoder::new(BufReader::with_capacity(BUFREADER_SIZE, stream));

            if op.dst_extents.len() != 1 {
                reporter.on_warning(
                    partition_name,
                    operation_index,
                    "Multi-extent Zstd not supported".to_string(),
                );
                return Ok(());
            }

            ctx.out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * ctx.block_size,
                ))
                .await?;
            match copy_with_buffer(&mut decoder, ctx.out_file, ctx.copy_buffer).await {
                Ok(_) => {}
                Err(e) => {
                    reporter.on_warning(
                        partition_name,
                        operation_index,
                        format!("Zstd decompression error: {}", e),
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Zero => {
            for ext in &op.dst_extents {
                ctx.out_file
                    .seek(std::io::SeekFrom::Start(
                        ext.start_block.unwrap_or(0) * ctx.block_size,
                    ))
                    .await?;
                for _ in 0..ext.num_blocks.unwrap_or(0) {
                    ctx.out_file.write_all(ctx.zero_buffer).await?;
                }
            }
        }
        install_operation::Type::SourceCopy
        | install_operation::Type::SourceBsdiff
        | install_operation::Type::BrotliBsdiff
        | install_operation::Type::Lz4diffBsdiff
        | install_operation::Type::Lz4diffPuffdiff
        | install_operation::Type::Puffdiff
        | install_operation::Type::Zucchini => {
            #[cfg(feature = "diff_ota")]
            {
                if let (Some(diff_ctx), Some(source_file)) =
                    (ctx.diff_ctx, ctx.source_file.as_mut())
                {
                    process_diff_operation(DiffOperationParams {
                        operation_index,
                        op,
                        ctx: diff_ctx,
                        partition_name,
                        source_file,
                        out_file: ctx.out_file,
                        payload_reader: ctx.payload_reader,
                        data_offset: ctx.data_offset,
                        reporter,
                    })
                    .await?;
                } else {
                    return Err(anyhow!(
                        "Operation {} is a differential OTA operation but source directory not provided. Use --source-dir option.",
                        operation_index
                    ));
                }
            }
            #[cfg(not(feature = "diff_ota"))]
            {
                return Err(anyhow!(
                    "Operation {} is a differential OTA operation. Rebuild with 'diff_ota' feature enabled to support incremental OTAs.",
                    operation_index
                ));
            }
        }
        _ => {
            reporter.on_warning(
                partition_name,
                operation_index,
                "Unknown operation type".to_string(),
            );
            return Ok(());
        }
    }
    Ok(())
}

/// dump a partition to disk
///
/// # Arguments
/// * `partition` -> the partition metadata
/// * `data_offset` -> offset in payload file where data begins
/// * `block_size` -> block size for the partition
/// * `output_path` -> where to write the partition image
/// * `payload_reader` -> reader for the payload data
/// * `reporter` -> progress reporter implementation
/// * `source_dir` -> (optional) directory containing source images for differential OTA
pub async fn dump_partition<P: AsyncPayloadRead>(
    partition: &PartitionUpdate,
    data_offset: u64,
    block_size: u64,
    output_path: PathBuf,
    payload_reader: &P,
    reporter: &dyn ProgressReporter,
    source_dir: Option<PathBuf>,
) -> Result<()> {
    let partition_name = &partition.partition_name;
    let total_ops = partition.operations.len() as u64;

    reporter.on_start(partition_name, total_ops);

    // Check if this is a differential OTA
    let has_diff_ops = partition
        .operations
        .iter()
        .any(|op| is_diff_operation(op.r#type()));

    #[cfg(feature = "diff_ota")]
    let (diff_ctx, mut source_file_opt) = if has_diff_ops {
        if let Some(src_dir) = source_dir {
            let diff_ctx = DiffContext::new(src_dir, block_size);
            let source_img_path = diff_ctx.source_dir.join(format!("{}.img", partition_name));

            if !source_img_path.exists() {
                return Err(anyhow!(
                    "Differential OTA requires source image: {} not found",
                    source_img_path.display()
                ));
            }

            let source_file = File::open(&source_img_path).await?;
            (Some(diff_ctx), Some(source_file))
        } else {
            return Err(anyhow!(
                "Partition '{}' contains differential operations but no source directory provided.",
                partition_name
            ));
        }
    } else {
        (None, None)
    };

    #[cfg(not(feature = "diff_ota"))]
    if has_diff_ops && source_dir.is_none() {
        return Err(anyhow!(
            "Partition '{}' contains differential operations. Rebuild with 'diff_ota' feature or provide source directory.",
            partition_name
        ));
    }

    let mut out_file = File::create(&output_path).await?;

    if let Some(info) = &partition.new_partition_info {
        if let Some(size) = info.size {
            out_file.set_len(size).await?;
        } else {
            return Err(anyhow!("Partition size is missing"));
        }
    }

    let mut reader = payload_reader.open_reader().await?;

    // allocate reusable buffers once
    let mut copy_buffer = vec![0u8; COPY_BUFFER_SIZE];
    let zero_buffer = vec![0u8; block_size as usize];

    // Create context to group related parameters
    let mut ctx = OperationContext {
        data_offset,
        block_size,
        payload_reader: &mut *reader,
        out_file: &mut out_file,
        copy_buffer: &mut copy_buffer,
        zero_buffer: &zero_buffer,
        #[cfg(feature = "diff_ota")]
        diff_ctx: diff_ctx.as_ref(),
        #[cfg(feature = "diff_ota")]
        source_file: source_file_opt.as_mut(),
    };

    for (i, op) in partition.operations.iter().enumerate() {
        // Check for cancellation before processing each operation
        if reporter.is_cancelled() {
            return Err(anyhow!("Extraction cancelled by user"));
        }

        process_operation_streaming(i, op, &mut ctx, reporter, partition_name).await?;
        reporter.on_progress(partition_name, (i + 1) as u64, total_ops);
    }

    out_file.flush().await?;

    reporter.on_complete(partition_name, total_ops);

    Ok(())
}
