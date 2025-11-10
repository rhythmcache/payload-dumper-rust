use anyhow::{Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

use crate::args::Args;
use crate::payload::payload_dumper::{AsyncPayloadRead, dump_partition};
use crate::readers::local_reader::LocalAsyncPayloadReader;
use crate::readers::remote_zip_reader::RemoteAsyncZipPayloadReader;
use crate::utils::format_elapsed_time;
use crate::verify::verify_partitions_hash;
use crate::{DeltaArchiveManifest, PartitionUpdate};

/// information about the data range needed for a partition
#[derive(Debug, Clone)]
pub struct PartitionDataRange {
    pub min_offset: u64,
    // pub max_offset: u64,
    pub total_bytes: u64,
    // pub operation_count: usize,
}

/// calculate the min/max data offsets for all operations in a partition
pub fn calculate_partition_range(
    partition: &PartitionUpdate,
    data_offset: u64,
) -> Option<PartitionDataRange> {
    let mut min_offset = u64::MAX;
    let mut max_offset = 0u64;
    let mut ops_with_data = 0;

    for op in &partition.operations {
        // only consider operations that actually read from payload data
        if let (Some(offset), Some(length)) = (op.data_offset, op.data_length) {
            if length > 0 {
                let abs_offset = data_offset + offset;
                let end_offset = abs_offset + length;

                min_offset = min_offset.min(abs_offset);
                max_offset = max_offset.max(end_offset);
                ops_with_data += 1;
            }
        }
    }

    if ops_with_data == 0 || min_offset == u64::MAX {
        return None;
    }

    Some(PartitionDataRange {
        min_offset,
        //  max_offset,
        total_bytes: max_offset - min_offset,
        //  operation_count: ops_with_data,
    })
}

async fn download_partition_data_to_path(
    zip_reader: &RemoteAsyncZipPayloadReader,
    range: &PartitionDataRange,
    temp_dir_path: &PathBuf,
    partition_name: &str,
    progress_bar: &ProgressBar,
) -> Result<PathBuf> {
    progress_bar.set_message(format!(
        "Downloading {} ({:.2} MB)",
        partition_name,
        range.total_bytes as f64 / 1024.0 / 1024.0
    ));

    let temp_path = temp_dir_path.join(format!("{}.prefetch", partition_name));
    let mut file = File::create(&temp_path).await?;

    let mut stream = zip_reader
        .stream_from(range.min_offset, range.total_bytes)
        .await?;

    const BUFFER_SIZE: usize = 256 * 1024; // 256 KB buffer for reading
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut downloaded = 0u64;
    let total = range.total_bytes;

    loop {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            break; // end of stream
        }

        // write what we read
        file.write_all(&buffer[..n]).await?;

        downloaded += n as u64;
        let percent = (downloaded as f64 / total as f64 * 100.0) as u64;
        progress_bar.set_position(percent);
    }

    file.flush().await?;
    drop(file);

    progress_bar.finish_with_message(format!("✓ Downloaded {}", partition_name));

    Ok(temp_path)
}

/// wrapper reader that translates offsets from absolute to relative
struct OffsetTranslatingReader {
    inner: LocalAsyncPayloadReader,
    base_offset: u64,
}

impl OffsetTranslatingReader {
    async fn new(path: PathBuf, base_offset: u64) -> Result<Self> {
        let inner = LocalAsyncPayloadReader::new(path).await?;
        Ok(Self { inner, base_offset })
    }
}

#[async_trait::async_trait]
impl AsyncPayloadRead for OffsetTranslatingReader {
    async fn stream_from(
        &self,
        offset: u64,
        length: u64,
    ) -> Result<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>> {
        // translate absolute offset to relative offset in temp file
        if offset < self.base_offset {
            return Err(anyhow!(
                "Offset {} is before base offset {}",
                offset,
                self.base_offset
            ));
        }

        let relative_offset = offset - self.base_offset;
        self.inner.stream_from(relative_offset, length).await
    }
}

pub async fn prefetch_and_extract(
    url: String,
    manifest: DeltaArchiveManifest,
    data_offset: u64,
    args: Arc<Args>,
    partitions_to_extract: Vec<PartitionUpdate>,
    multi_progress: Arc<MultiProgress>,
) -> Result<()> {
    let start_time = Instant::now();

    let main_pb = multi_progress.add(ProgressBar::new_spinner());
    main_pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    main_pb.enable_steady_tick(tokio::time::Duration::from_millis(300));

    main_pb.set_message("Initializing prefetch mode...");

    // mktmp
    let temp_dir = TempDir::new()?;
    main_pb.println(format!(
        "- Created temporary directory: {:?}",
        temp_dir.path()
    ));

    // calculate ranges for all partitions
    let mut partition_info: HashMap<String, PartitionDataRange> = HashMap::new();
    let mut total_download_size = 0u64;

    for partition in &partitions_to_extract {
        if let Some(range) = calculate_partition_range(partition, data_offset) {
            total_download_size += range.total_bytes;
            partition_info.insert(partition.partition_name.clone(), range);
        }
    }

    main_pb.println(format!(
        "- Total data to download: {:.2} MB across {} partitions",
        total_download_size as f64 / 1024.0 / 1024.0,
        partition_info.len()
    ));

    // calculate thread count (same logic as main.rs)
    let thread_count = if args.no_parallel {
        1
    } else if let Some(threads) = args.threads {
        threads
    } else {
        num_cpus::get()
    };

    // get block size for extraction
    let block_size = manifest.block_size.unwrap_or(4096) as u64;

    // download and extract partitions as soon as each download completes
    main_pb.set_message("Downloading and extracting partitions...");

    let download_semaphore = Arc::new(Semaphore::new(thread_count));
    let extract_semaphore = Arc::new(Semaphore::new(thread_count));
    let mut combined_tasks = Vec::new();

    for partition in &partitions_to_extract {
        let partition_name = partition.partition_name.clone();

        if let Some(range) = partition_info.get(&partition_name) {
            let range = range.clone();
            let temp_dir_path = temp_dir.path().to_path_buf();
            let partition = partition.clone();
            let args = Arc::clone(&args);
            let multi_progress = Arc::clone(&multi_progress);

            let download_pb = multi_progress.add(ProgressBar::new(100));
            download_pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/white}] {percent}% - {msg}")
                    .unwrap()
                    .progress_chars("▰▱ "),
            );
            download_pb.enable_steady_tick(tokio::time::Duration::from_secs(1));

            let url = url.clone();
            let user_agent = args.user_agent.clone();
            let download_semaphore = Arc::clone(&download_semaphore);
            let extract_semaphore = Arc::clone(&extract_semaphore);

            // spawn combined download + extract task
            let task = tokio::spawn(async move {
                // download
                let temp_path = {
                    let _permit = download_semaphore.acquire().await.unwrap();

                    let zip_reader = RemoteAsyncZipPayloadReader::new(url, user_agent.as_deref())
                        .await
                        .map_err(|e| (partition_name.clone(), e))?;

                    let temp_path = download_partition_data_to_path(
                        &zip_reader,
                        &range,
                        &temp_dir_path,
                        &partition_name,
                        &download_pb,
                    )
                    .await
                    .map_err(|e| (partition_name.clone(), e))?;

                    // release download permit before extraction
                    drop(_permit);
                    temp_path
                };

                // extract immediately after download completes
                let _permit = extract_semaphore.acquire().await.unwrap();

                let reader = OffsetTranslatingReader::new(temp_path, range.min_offset)
                    .await
                    .map(|r| Arc::new(r) as Arc<dyn AsyncPayloadRead>)
                    .map_err(|e| (partition_name.clone(), e))?;

                dump_partition(
                    &partition,
                    data_offset,
                    block_size,
                    &args,
                    &reader,
                    Some(&multi_progress),
                )
                .await
                .map_err(|e| (partition_name, e))
            });

            combined_tasks.push(task);
        }
    }

    // wait for all download+extract tasks to complete
    let results = futures::future::join_all(combined_tasks).await;

    let mut failed_partitions = Vec::new();
    for result in results {
        match result {
            Ok(Ok(())) => {}
            Ok(Err((partition_name, error))) => {
                eprintln!("Failed to process partition {}: {}", partition_name, error);
                failed_partitions.push(partition_name);
            }
            Err(e) => {
                eprintln!("Task panicked: {}", e);
            }
        }
    }

    main_pb.println("✓ All partitions downloaded and extracted");

    if !args.no_verify {
        main_pb.println("- Verifying partition hashes...");

        let partitions_to_verify: Vec<&PartitionUpdate> = partitions_to_extract
            .iter()
            .filter(|p| !failed_partitions.contains(&p.partition_name))
            .collect();

        match verify_partitions_hash(&partitions_to_verify, &args, &multi_progress).await {
            Ok(failed_verifications) => {
                if !failed_verifications.is_empty() {
                    eprintln!(
                        "Hash verification failed for {} partitions.",
                        failed_verifications.len()
                    );
                    failed_partitions.extend(failed_verifications);
                }
            }
            Err(e) => {
                eprintln!("Error during hash verification: {}", e);
            }
        }
    } else {
        main_pb.println("- Skipping hash verification");
    }

    let elapsed_time = format_elapsed_time(start_time.elapsed());

    if failed_partitions.is_empty() {
        main_pb.finish_with_message(format!(
            "All partitions extracted successfully! (in {})",
            elapsed_time
        ));
    } else {
        main_pb.finish_with_message(format!(
            "Completed with {} failed partitions. (in {})",
            failed_partitions.len(),
            elapsed_time
        ));
        println!(
            "\nExtraction completed with {} failed partitions in {}. Output directory: {:?}",
            failed_partitions.len(),
            elapsed_time,
            args.out
        );
    }

    Ok(())
}
