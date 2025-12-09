// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncWriteExt};

use crate::PartitionUpdate;
use crate::http::HttpReader;
use crate::payload::payload_dumper::{AsyncPayloadRead, PayloadReader, ProgressReporter};
use crate::readers::local_reader::LocalAsyncPayloadReader;

/// configuration for partition extraction
#[derive(Debug, Clone)]
pub struct PartitionExtractionConfig {
    pub data_offset: u64,
    pub block_size: u64,
    pub payload_offset: u64,
}

/// paths used during extraction
pub struct ExtractionPaths {
    pub temp_path: PathBuf,
    pub output_path: PathBuf,
}

/// information about the data range needed for a partition
#[derive(Debug, Clone)]
pub struct PartitionDataRange {
    pub min_offset: u64,
    pub total_bytes: u64,
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
        if let (Some(offset), Some(length)) = (op.data_offset, op.data_length)
            && length > 0
        {
            let abs_offset = data_offset + offset;
            let end_offset = abs_offset + length;

            min_offset = min_offset.min(abs_offset);
            max_offset = max_offset.max(end_offset);
            ops_with_data += 1;
        }
    }

    if ops_with_data == 0 || min_offset == u64::MAX {
        return None;
    }

    Some(PartitionDataRange {
        min_offset,
        total_bytes: max_offset - min_offset,
    })
}

/// progress reporter for download operations
#[async_trait]
pub trait DownloadProgressReporter: Send + Sync {
    /// called when download starts
    fn on_download_start(&self, partition_name: &str, total_bytes: u64);

    /// called during download progress
    fn on_download_progress(&self, partition_name: &str, downloaded: u64, total: u64);

    /// called when download completes
    fn on_download_complete(&self, partition_name: &str, total_bytes: u64);
}

/// no-op download reporter
pub struct NoOpDownloadReporter;

impl DownloadProgressReporter for NoOpDownloadReporter {
    fn on_download_start(&self, _: &str, _: u64) {}
    fn on_download_progress(&self, _: &str, _: u64, _: u64) {}
    fn on_download_complete(&self, _: &str, _: u64) {}
}

/// download partition data from HTTP to a temporary file
///
/// # arguments
/// * `payload_offset` - offset where payload.bin starts in the file (0 for .bin, non-zero for ZIP)
async fn download_partition_data(
    http_reader: &HttpReader,
    range: &PartitionDataRange,
    temp_path: &PathBuf,
    partition_name: &str,
    reporter: &dyn DownloadProgressReporter,
    payload_offset: u64,
) -> Result<()> {
    reporter.on_download_start(partition_name, range.total_bytes);

    let mut file = File::create(temp_path).await?;

    const BUFFER_SIZE: usize = 256 * 1024; // 256 KB buffer
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut downloaded = 0u64;
    let total = range.total_bytes;
    let mut current_offset = range.min_offset + payload_offset;

    while downloaded < total {
        let remaining = total - downloaded;
        let chunk_size = remaining.min(BUFFER_SIZE as u64) as usize;

        http_reader
            .read_at(current_offset, &mut buffer[..chunk_size])
            .await?;

        file.write_all(&buffer[..chunk_size]).await?;

        downloaded += chunk_size as u64;
        current_offset += chunk_size as u64;

        reporter.on_download_progress(partition_name, downloaded, total);
    }

    file.flush().await?;
    drop(file);

    reporter.on_download_complete(partition_name, total);

    Ok(())
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

#[async_trait]
impl AsyncPayloadRead for OffsetTranslatingReader {
    async fn open_reader(&self) -> Result<Box<dyn PayloadReader>> {
        let inner_reader = self.inner.open_reader().await?;
        Ok(Box::new(OffsetTranslatingPayloadReader {
            inner: inner_reader,
            base_offset: self.base_offset,
        }))
    }
}

struct OffsetTranslatingPayloadReader {
    inner: Box<dyn PayloadReader>,
    base_offset: u64,
}

#[async_trait]
impl PayloadReader for OffsetTranslatingPayloadReader {
    async fn read_range(
        &mut self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send + '_>>> {
        if offset < self.base_offset {
            return Err(anyhow!(
                "Offset {} is before base offset {}",
                offset,
                self.base_offset
            ));
        }

        let relative_offset = offset - self.base_offset;
        self.inner.read_range(relative_offset, length).await
    }
}

/// prefetch and extract a single partition
///
/// downloads the required data range from remote source to a temporary file,
/// then extracts the partition using the standard dump_partition function.
///
/// # arguments
/// * `partition` - the partition to extract
/// * `config` - extraction configuration (data_offset, block_size, payload_offset)
/// * `http_reader` - HTTP reader for downloading data
/// * `paths` - temporary and output file paths
/// * `download_reporter` - reporter for download progress
/// * `extract_reporter` - reporter for extraction progress
pub async fn prefetch_and_dump_partition<D, E>(
    partition: &PartitionUpdate,
    config: &PartitionExtractionConfig,
    http_reader: &HttpReader,
    paths: ExtractionPaths,
    download_reporter: &D,
    extract_reporter: &E,
) -> Result<()>
where
    D: DownloadProgressReporter,
    E: ProgressReporter,
{
    let partition_name = &partition.partition_name;

    // calculate the data range needed for this partition
    let range = calculate_partition_range(partition, config.data_offset)
        .ok_or_else(|| anyhow!("Partition {} has no data to extract", partition_name))?;

    // download partition data
    download_partition_data(
        http_reader,
        &range,
        &paths.temp_path,
        partition_name,
        download_reporter,
        config.payload_offset,
    )
    .await?;

    // create offset-translating reader
    let reader = OffsetTranslatingReader::new(paths.temp_path, range.min_offset).await?;

    // extract using standard dump_partition
    crate::payload::payload_dumper::dump_partition(
        partition,
        config.data_offset,
        config.block_size,
        paths.output_path,
        &reader,
        extract_reporter,
    )
    .await?;

    Ok(())
}
