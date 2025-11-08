use std::path::PathBuf;
use std::sync::Arc;
use std::pin::Pin;
use anyhow::{Result, anyhow};
use async_compression::tokio::bufread::{BzDecoder, XzDecoder, ZstdDecoder};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, AsyncRead, BufReader};
use tokio::sync::Semaphore;
pub use crate::PartitionUpdate;
use crate::args::Args;
use crate::{InstallOperation, install_operation};

pub struct AsyncPayloadReader {
    path: PathBuf,
    semaphore: Arc<Semaphore>,
}

impl AsyncPayloadReader {
    pub async fn new(path: PathBuf) -> Result<Self> {
        File::open(&path).await?;
        let max_concurrent_reads = num_cpus::get() * 2;
        Ok(Self {
            path,
            semaphore: Arc::new(Semaphore::new(max_concurrent_reads)),
        })
    }

    pub async fn stream_from(&self, offset: u64, length: u64) -> Result<StreamingFileReader> {
        let permit = self.semaphore.clone().acquire_owned().await?;
        let mut file = File::open(&self.path).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        Ok(StreamingFileReader {
            file: BufReader::new(file),
            remaining: length,
            _permit: permit,
        })
    }

}

pub struct StreamingFileReader {
    file: BufReader<File>,
    remaining: u64,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl AsyncRead for StreamingFileReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.remaining == 0 {
            return std::task::Poll::Ready(Ok(()));
        }
        let max_read = std::cmp::min(buf.remaining() as u64, self.remaining) as usize;
        let mut limited_buf = buf.take(max_read);
        let pin = Pin::new(&mut self.file);
        match pin.poll_read(cx, &mut limited_buf) {
            std::task::Poll::Ready(Ok(())) => {
                let filled = limited_buf.filled().len();
                self.remaining -= filled as u64;
                buf.advance(filled);
                std::task::Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

async fn process_operation_streaming(
    operation_index: usize,
    op: &InstallOperation,
    data_offset: u64,
    block_size: u64,
    payload_reader: &AsyncPayloadReader,
    out_file: &mut File,
) -> Result<()> {
    let offset = data_offset + op.data_offset.unwrap_or(0);
    let length = op.data_length.unwrap_or(0);
    
    match op.r#type() {
        install_operation::Type::Replace => {
            let mut stream = payload_reader.stream_from(offset, length).await?;
            out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                ))
                .await?;
            tokio::io::copy(&mut stream, out_file).await?;
        }
        install_operation::Type::ReplaceXz => {
            let stream = payload_reader.stream_from(offset, length).await?;
            let mut decoder = XzDecoder::new(BufReader::new(stream));
            out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                ))
                .await?;
            match tokio::io::copy(&mut decoder, out_file).await {
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
            let stream = payload_reader.stream_from(offset, length).await?;
            let mut decoder = BzDecoder::new(BufReader::new(stream));
            out_file
                .seek(std::io::SeekFrom::Start(
                    op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                ))
                .await?;
            match tokio::io::copy(&mut decoder, out_file).await {
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
            let stream = payload_reader.stream_from(offset, length).await?;
            let mut decoder = ZstdDecoder::new(BufReader::new(stream));
            if op.dst_extents.len() == 1 {
                out_file
                    .seek(std::io::SeekFrom::Start(
                        op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                    ))
                    .await?;
                match tokio::io::copy(&mut decoder, out_file).await {
                    Ok(_) => {}
                    Err(e) => {
                        println!(
                            "  Warning: Skipping operation {} due to Zstd decompression error: {}",
                            operation_index, e
                        );
                        return Ok(());
                    }
                }
            } else {
                let mut decompressed = Vec::new();
                match decoder.read_to_end(&mut decompressed).await {
                    Ok(_) => {
                        let mut pos = 0;
                        for ext in &op.dst_extents {
                            let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                            let end_pos = pos + ext_size;
                            if end_pos <= decompressed.len() {
                                out_file
                                    .seek(std::io::SeekFrom::Start(
                                        ext.start_block.unwrap_or(0) * block_size,
                                    ))
                                    .await?;
                                out_file.write_all(&decompressed[pos..end_pos]).await?;
                                pos = end_pos;
                            } else {
                                println!(
                                    "  Warning: Skipping extent in operation {} due to insufficient data.",
                                    operation_index
                                );
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        println!(
                            "  Warning: Skipping operation {} due to Zstd error: {}",
                            operation_index, e
                        );
                        return Ok(());
                    }
                }
            }
        }
        install_operation::Type::Zero => {
            let zeros = vec![0u8; block_size as usize];
            for ext in &op.dst_extents {
                out_file
                    .seek(std::io::SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))
                    .await?;
                for _ in 0..ext.num_blocks.unwrap_or(0) {
                    out_file.write_all(&zeros).await?;
                }
            }
        }
        install_operation::Type::SourceCopy
        | install_operation::Type::SourceBsdiff
        | install_operation::Type::BrotliBsdiff
        | install_operation::Type::Lz4diffBsdiff => {
            return Err(anyhow!(
                "Operation {} is a differential OTA operation which is not supported. Please use a full OTA package.",
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

pub async fn dump_partition(
    partition: &PartitionUpdate,
    data_offset: u64,
    block_size: u64,
    args: &Args,
    payload_reader: &AsyncPayloadReader,
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
                .progress_chars("▰▱△"),
        );
        pb.enable_steady_tick(tokio::time::Duration::from_millis(500));
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

    for (i, op) in partition.operations.iter().enumerate() {
        process_operation_streaming(
            i,
            op,
            data_offset,
            block_size,
            payload_reader,
            &mut out_file,
        )
        .await?;
        
        if let Some(pb) = &progress_bar {
            let percentage = ((i + 1) as f64 / total_ops as f64 * 100.0) as u64;
            pb.set_position(percentage);
        }
    }
    
    out_file.flush().await?;
    
    if let Some(pb) = progress_bar {
        pb.finish_with_message(format!("✓ Completed {} ({} ops)", partition_name, total_ops));
    }
    
    Ok(())
}