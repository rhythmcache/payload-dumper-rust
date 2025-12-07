use anyhow::{Result, anyhow};
use async_compression::tokio::bufread::{BzDecoder, XzDecoder, ZstdDecoder};
use async_trait::async_trait;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::pin::Pin;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};

pub use crate::PartitionUpdate;
use crate::args::Args;
use crate::{InstallOperation, install_operation};

const BUFREADER_SIZE: usize = 64 * 1024; // 64 KB for BufReader (decompression streams)
const COPY_BUFFER_SIZE: usize = 128 * 1024; // 128 KB for direct copy operations

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

async fn process_operation_streaming(
    operation_index: usize,
    op: &InstallOperation,
    data_offset: u64,
    block_size: u64,
    payload_reader: &mut dyn PayloadReader,
    out_file: &mut File,
    copy_buffer: &mut [u8],
    zero_buffer: &[u8],
) -> Result<()> {
    let offset = data_offset + op.data_offset.unwrap_or(0);
    let length = op.data_length.unwrap_or(0);

    match op.r#type() {
        install_operation::Type::Replace => {
            let mut stream = payload_reader.read_range(offset, length).await?;
            out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                ))
                .await?;
            copy_with_buffer(&mut stream, out_file, copy_buffer).await?;
        }
        install_operation::Type::ReplaceXz => {
            let stream = payload_reader.read_range(offset, length).await?;
            let mut decoder = XzDecoder::new(BufReader::with_capacity(BUFREADER_SIZE, stream));
            out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                ))
                .await?;
            match copy_with_buffer(&mut decoder, out_file, copy_buffer).await {
                Ok(_) => {}
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to XZ decompression error: {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::ReplaceBz => {
            let stream = payload_reader.read_range(offset, length).await?;
            let mut decoder = BzDecoder::new(BufReader::with_capacity(BUFREADER_SIZE, stream));
            out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                ))
                .await?;
            match copy_with_buffer(&mut decoder, out_file, copy_buffer).await {
                Ok(_) => {}
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to BZ2 decompression error: {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Zstd => {
            let stream = payload_reader.read_range(offset, length).await?;
            let mut decoder = ZstdDecoder::new(BufReader::with_capacity(BUFREADER_SIZE, stream));

            if op.dst_extents.len() != 1 {
                println!(
                    "  Warning: Skipping operation {} - multi-extent Zstd not supported",
                    operation_index
                );
                return Ok(());
            }

            out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                ))
                .await?;
            match copy_with_buffer(&mut decoder, out_file, copy_buffer).await {
                Ok(_) => {}
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to Zstd decompression error: {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Zero => {
            for ext in &op.dst_extents {
                out_file
                    .seek(std::io::SeekFrom::Start(
                        ext.start_block.unwrap_or(0) * block_size,
                    ))
                    .await?;
                for _ in 0..ext.num_blocks.unwrap_or(0) {
                    out_file.write_all(zero_buffer).await?;
                }
            }
        }
        install_operation::Type::SourceCopy
        | install_operation::Type::SourceBsdiff
        | install_operation::Type::BrotliBsdiff
        | install_operation::Type::Lz4diffBsdiff => {
            return Err(anyhow!(
                "Operation {} is a differential OTA operation which is not supported",
                operation_index
            ));
        }
        _ => {
            println!(
                "  Warning: Skipping operation {} due to unknown operation type",
                operation_index
            );
            return Ok(());
        }
    }
    Ok(())
}

pub async fn dump_partition<P: AsyncPayloadRead>(
    partition: &PartitionUpdate,
    data_offset: u64,
    block_size: u64,
    args: &Args,
    payload_reader: &P,
    multi_progress: Option<&MultiProgress>,
) -> Result<()> {
    let partition_name = &partition.partition_name;
    let total_ops = partition.operations.len() as u64;

    let progress_bar = if let Some(mp) = multi_progress {
        let pb = mp.add(ProgressBar::new(100));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/white}] {percent}% - {msg}")
                .unwrap()
                .progress_chars("▰▱ "),
        );
        pb.enable_steady_tick(tokio::time::Duration::from_secs(1));
        pb.set_message(format!("Processing {} ({} ops)", partition_name, total_ops));
        Some(pb)
    } else {
        None
    };

    let out_dir = &args.out;
    if args.out.to_string_lossy() != "-" {
        tokio::fs::create_dir_all(out_dir).await?;
    }

    let out_path = out_dir.join(format!("{}.img", partition_name));
    let mut out_file = File::create(&out_path).await?;

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

    for (i, op) in partition.operations.iter().enumerate() {
        process_operation_streaming(
            i,
            op,
            data_offset,
            block_size,
            &mut *reader,
            &mut out_file,
            &mut copy_buffer,
            &zero_buffer,
        )
        .await?;

        if let Some(pb) = &progress_bar {
            let percentage = ((i + 1) as f64 / total_ops as f64 * 100.0) as u64;
            pb.set_position(percentage);
        }
    }

    out_file.flush().await?;

    if let Some(pb) = progress_bar {
        pb.finish_with_message(format!(
            "✓ Completed {} ({} ops)",
            partition_name, total_ops
        ));
    }

    Ok(())
}
