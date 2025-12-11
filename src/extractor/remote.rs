// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

//! high-level API for remote payload dumper operations
use crate::metadata::get_metadata;
use crate::payload::payload_dumper::{ProgressReporter, dump_partition};
use crate::payload::payload_parser::{parse_remote_bin_payload, parse_remote_payload};
use crate::readers::remote_bin_reader::RemoteAsyncBinPayloadReader;
use crate::readers::remote_zip_reader::RemoteAsyncZipPayloadReader;
use anyhow::{Result, anyhow};
use std::path::Path;

// Re-export shared types from local module
pub use crate::extractor::local::{
    ExtractionProgress, ExtractionStatus, PartitionInfo, PayloadSummary, ProgressCallback, RUNTIME,
};

/// return type for remote partition listing that includes HTTP content length
#[derive(Debug, Clone)]
pub struct RemotePartitionList {
    pub json: String,
    pub content_length: u64,
}

/* List Partitions (Remote ZIP) */

/// list all partitions in a remote ZIP file containing payload.bin
///
/// Returns both the JSON summary and the HTTP content length of the file.
///
/// # Arguments
/// * `url` - URL to the remote ZIP file
/// * `user_agent` - Optional custom user agent string
///
/// # Panics
/// panics if called from within an async runtime context. This is a blocking
/// function and must be called from a synchronous context.
pub fn list_partitions_remote_zip(
    url: String,
    user_agent: Option<&str>,
) -> Result<RemotePartitionList> {
    // check if we're already inside a tokio runtime
    if tokio::runtime::Handle::try_current().is_ok() {
        panic!(
            "list_partitions_remote_zip cannot be called from within an async runtime. Use the async version or call from a blocking context."
        );
    }

    RUNTIME.block_on(async {
        let (manifest, data_offset, content_length) = parse_remote_payload(url, user_agent).await?;

        // use get_metadata with full_mode=false for basic info
        let metadata = get_metadata(&manifest, data_offset, false, None).await?;

        let partitions: Vec<PartitionInfo> = metadata
            .partitions
            .iter()
            .map(|p| PartitionInfo {
                name: p.partition_name.clone(),
                size_bytes: p.size_in_bytes,
                size_readable: p.size_readable.clone(),
                operations_count: p.operations_count,
                compression_type: p.compression_type.clone(),
                hash: p.hash.clone(),
            })
            .collect();

        let summary = PayloadSummary {
            total_partitions: partitions.len(),
            total_operations: metadata.total_operations_count,
            total_size_bytes: partitions.iter().map(|p| p.size_bytes).sum(),
            total_size_readable: crate::utils::format_size(
                partitions.iter().map(|p| p.size_bytes).sum(),
            ),
            partitions,
        };

        let json = serde_json::to_string_pretty(&summary)
            .map_err(|e| anyhow!("Failed to serialize partitions: {}", e))?;

        Ok(RemotePartitionList {
            json,
            content_length,
        })
    })
}

/* List Partitions (Remote .bin) */

/// list all partitions in a remote payload.bin file (not in ZIP)
///
/// Returns both the JSON summary and the HTTP content length of the file.
///
/// # arguments
/// * `url` - URL to the remote payload.bin file
/// * `user_agent` - Optional custom user agent string
///
/// # Panics
/// panics if called from within an async runtime context. This is a blocking
/// function and must be called from a synchronous context.
pub fn list_partitions_remote_bin(
    url: String,
    user_agent: Option<&str>,
) -> Result<RemotePartitionList> {
    // check if we're already inside a tokio runtime
    if tokio::runtime::Handle::try_current().is_ok() {
        panic!(
            "list_partitions_remote_bin cannot be called from within an async runtime. Use the async version or call from a blocking context."
        );
    }

    RUNTIME.block_on(async {
        let (manifest, data_offset, content_length) =
            parse_remote_bin_payload(url, user_agent).await?;

        // use get_metadata with full_mode=false for basic info
        let metadata = get_metadata(&manifest, data_offset, false, None).await?;

        let partitions: Vec<PartitionInfo> = metadata
            .partitions
            .iter()
            .map(|p| PartitionInfo {
                name: p.partition_name.clone(),
                size_bytes: p.size_in_bytes,
                size_readable: p.size_readable.clone(),
                operations_count: p.operations_count,
                compression_type: p.compression_type.clone(),
                hash: p.hash.clone(),
            })
            .collect();

        let summary = PayloadSummary {
            total_partitions: partitions.len(),
            total_operations: metadata.total_operations_count,
            total_size_bytes: partitions.iter().map(|p| p.size_bytes).sum(),
            total_size_readable: crate::utils::format_size(
                partitions.iter().map(|p| p.size_bytes).sum(),
            ),
            partitions,
        };

        let json = serde_json::to_string_pretty(&summary)
            .map_err(|e| anyhow!("Failed to serialize partitions: {}", e))?;

        Ok(RemotePartitionList {
            json,
            content_length,
        })
    })
}

/* Extract Partition (Remote ZIP) */

/// extract a single partition from a remote ZIP file containing payload.bin
///
/// this function extracts a single partition. For parallel extraction of multiple
/// partitions, call this function from multiple threads (one per partition).
/// the caller is responsible for managing parallelization and thread limits.
///
/// # arguments
/// * `url` - URL to the remote ZIP file
/// * `partition_name` - Name of the partition to extract
/// * `output_path` - Local path where to write the partition image
/// * `user_agent` - Optional custom user agent string
/// * `progress_callback` - Optional callback for progress updates
///
/// # Thread Safety
/// this function can be safely called concurrently from multiple threads thanks
/// to the shared multi-threaded runtime.
///
/// # Cancellation
/// if a progress callback is provided and returns `false`, the extraction will
/// be cancelled. note that cancellation may not be immediate - it depends on
/// when the next progress callback is triggered.
///
/// # Panics
/// panics if called from within an async runtime context (e.g inside a tokio task).
/// this is a blocking function and must be called from a synchronous context only.
///
/// # Example
/// ```no_run
/// use std::thread;
///
/// // Correct: Call from multiple OS threads
/// let url = "https://example.com/payload.zip".to_string();
/// let handles: Vec<_> = vec!["system", "vendor", "boot"]
///     .into_iter()
///     .map(|partition| {
///         let url = url.clone();
///         thread::spawn(move || {
///             extract_partition_remote_zip(
///                 url,
///                 partition,
///                 format!("out/{}", partition),
///                 None,
///                 None
///             )
///         })
///     })
///     .collect();
///
/// for handle in handles {
///     handle.join().unwrap().unwrap();
/// }
/// ```
pub fn extract_partition_remote_zip<P: AsRef<Path>>(
    url: String,
    partition_name: &str,
    output_path: P,
    user_agent: Option<&str>,
    progress_callback: Option<ProgressCallback>,
) -> Result<()> {
    // check if we're already inside a tokio runtime
    if tokio::runtime::Handle::try_current().is_ok() {
        panic!(
            "extract_partition_remote_zip cannot be called from within an async runtime. Use spawn_blocking or call from a synchronous context."
        );
    }

    RUNTIME.block_on(async {
        let (manifest, data_offset, _content_length) =
            parse_remote_payload(url.clone(), user_agent).await?;

        let partition = manifest
            .partitions
            .iter()
            .find(|p| p.partition_name == partition_name)
            .ok_or_else(|| anyhow!("Partition '{}' not found in payload", partition_name))?;

        let block_size = manifest.block_size.unwrap_or(4096) as u64;

        if let Some(parent) = output_path.as_ref().parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let payload_reader = RemoteAsyncZipPayloadReader::new(url, user_agent).await?;

        let reporter: Box<dyn ProgressReporter> = if let Some(callback) = progress_callback {
            Box::new(crate::extractor::local::CallbackProgressReporter::new(
                callback,
            ))
        } else {
            Box::new(crate::payload::payload_dumper::NoOpReporter)
        };

        dump_partition(
            partition,
            data_offset,
            block_size,
            output_path.as_ref().to_path_buf(),
            &payload_reader,
            &*reporter,
        )
        .await?;

        Ok(())
    })
}

/* Extract Partition (Remote .bin) */

/// extract a single partition from a remote payload.bin file (not in ZIP)
///
/// this function extracts a single partition. For parallel extraction of multiple
/// partitions, call this function from multiple threads (one per partition).
/// the caller is responsible for managing parallelization and thread limits.
///
/// # Arguments
/// * `url` - URL to the remote payload.bin file
/// * `partition_name` - Name of the partition to extract
/// * `output_path` - Local path where to write the partition image
/// * `user_agent` - Optional custom user agent string
/// * `progress_callback` - Optional callback for progress updates
///
/// # Thread Safety
/// this function can be safely called concurrently from multiple threads thanks
/// to the shared multi-threaded runtime.
///
/// # Cancellation
/// if a progress callback is provided and returns `false`, the extraction will
/// be cancelled. note that cancellation may not be immediate - it depends on
/// when the next progress callback is triggered.
///
/// # Panics
/// panics if called from within an async runtime context (e.g inside a tokio task).
/// this is a blocking function and must be called from a synchronous context only.
pub fn extract_partition_remote_bin<P: AsRef<Path>>(
    url: String,
    partition_name: &str,
    output_path: P,
    user_agent: Option<&str>,
    progress_callback: Option<ProgressCallback>,
) -> Result<()> {
    // check if we're already inside a tokio runtime
    if tokio::runtime::Handle::try_current().is_ok() {
        panic!(
            "extract_partition_remote_bin cannot be called from within an async runtime. Use spawn_blocking or call from a synchronous context."
        );
    }

    RUNTIME.block_on(async {
        let (manifest, data_offset, _content_length) =
            parse_remote_bin_payload(url.clone(), user_agent).await?;

        let partition = manifest
            .partitions
            .iter()
            .find(|p| p.partition_name == partition_name)
            .ok_or_else(|| anyhow!("Partition '{}' not found in payload", partition_name))?;

        let block_size = manifest.block_size.unwrap_or(4096) as u64;

        if let Some(parent) = output_path.as_ref().parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let payload_reader = RemoteAsyncBinPayloadReader::new(url, user_agent).await?;

        let reporter: Box<dyn ProgressReporter> = if let Some(callback) = progress_callback {
            Box::new(crate::extractor::local::CallbackProgressReporter::new(
                callback,
            ))
        } else {
            Box::new(crate::payload::payload_dumper::NoOpReporter)
        };

        dump_partition(
            partition,
            data_offset,
            block_size,
            output_path.as_ref().to_path_buf(),
            &payload_reader,
            &*reporter,
        )
        .await?;

        Ok(())
    })
}
