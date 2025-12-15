// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

//! high-level API for payload dumper operations
use crate::metadata::get_metadata;
use crate::payload::payload_dumper::{ProgressReporter, dump_partition};
use crate::payload::payload_parser::parse_local_payload;
#[cfg(feature = "remote_zip")]
use crate::payload::payload_parser::parse_local_zip_payload;
use crate::readers::local_reader::LocalAsyncPayloadReader;
#[cfg(feature = "remote_zip")]
use crate::readers::local_zip_reader::LocalAsyncZipPayloadReader;
use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::runtime::Runtime;

/* Global Shared Runtime */

/// global shared tokio runtime for all blocking API calls.
///
/// uses multi-threaded runtime to support concurrent block_on() calls
/// from multiple threads. The caller is responsible for limiting parallelization
/// (e.g spawning only N threads for N partitions).
pub static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(num_cpus::get().max(2)) // use CPU count, minimum 2
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime")
});

/* Data Types */

/// progress information for partition extraction
#[derive(Debug, Clone)]
pub struct ExtractionProgress {
    pub partition_name: String,
    pub current_operation: u64,
    pub total_operations: u64,
    pub percentage: f64,
    pub status: ExtractionStatus,
}

#[derive(Debug, Clone)]
pub enum ExtractionStatus {
    Started,
    InProgress,
    Completed,
    Warning {
        operation_index: usize,
        message: String,
    },
}

/// callback type for progress updates
/// returns true to continue, false to cancel extraction
pub type ProgressCallback = Box<dyn Fn(ExtractionProgress) -> bool + Send + Sync>;

/// simple partition informations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PartitionInfo {
    pub name: String,
    pub size_bytes: u64,
    pub size_readable: String,
    pub operations_count: usize,
    pub compression_type: String,
    pub hash: Option<String>,
}

/// summary information about the payload
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PayloadSummary {
    pub partitions: Vec<PartitionInfo>,
    pub total_partitions: usize,
    pub total_operations: usize,
    pub total_size_bytes: u64,
    pub total_size_readable: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_patch_level: Option<String>,
}

/* List Partitions (Payload.bin) */

/// list all partitions in a payload.bin file with their basic information
///
/// # Panics
/// panics if called from within an async runtime context. This is a blocking
/// function and must be called from a synchronous context.
pub fn list_partitions<P: AsRef<Path>>(payload_path: P) -> Result<String> {
    // check if we're already inside a tokio runtime
    if tokio::runtime::Handle::try_current().is_ok() {
        panic!(
            "list_partitions cannot be called from within an async runtime. Use the async version or call from a blocking context."
        );
    }

    RUNTIME.block_on(async {
        let (manifest, data_offset) = parse_local_payload(payload_path.as_ref()).await?;

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
            security_patch_level: metadata.security_patch_level.clone(),
        };

        serde_json::to_string_pretty(&summary)
            .map_err(|e| anyhow!("Failed to serialize partitions: {}", e))
    })
}

/* List Partitions (ZIP file) */

/// list all partitions in a ZIP file containing payload.bin
///
/// # Panics
/// panics if called from within an async runtime context. this is a blocking
/// function and must be called from a synchronous context.
#[cfg(feature = "remote_zip")]
pub fn list_partitions_zip<P: AsRef<Path>>(zip_path: P) -> Result<String> {
    // check if we're already inside a tokio runtime
    if tokio::runtime::Handle::try_current().is_ok() {
        panic!(
            "list_partitions_zip cannot be called from within an async runtime. Use the async version or call from a blocking context."
        );
    }

    RUNTIME.block_on(async {
        let (manifest, data_offset) =
            parse_local_zip_payload(zip_path.as_ref().to_path_buf()).await?;

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
            security_patch_level: metadata.security_patch_level.clone(),
        };

        serde_json::to_string_pretty(&summary)
            .map_err(|e| anyhow!("Failed to serialize partitions: {}", e))
    })
}

/* Extract Partition */

pub struct CallbackProgressReporter {
    callback: Arc<ProgressCallback>,
    cancelled: Arc<AtomicBool>,
}

impl CallbackProgressReporter {
    pub fn new(callback: ProgressCallback) -> Self {
        Self {
            callback: Arc::new(callback),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ProgressReporter for CallbackProgressReporter {
    fn on_start(&self, partition_name: &str, total_operations: u64) {
        let progress = ExtractionProgress {
            partition_name: partition_name.to_string(),
            current_operation: 0,
            total_operations,
            percentage: 0.0,
            status: ExtractionStatus::Started,
        };

        // call without holding any lock -> callback is Send + Sync
        let should_continue = (self.callback)(progress);

        if !should_continue {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }

    fn on_progress(&self, partition_name: &str, current_op: u64, total_ops: u64) {
        let percentage = if total_ops > 0 {
            (current_op as f64 / total_ops as f64) * 100.0
        } else {
            0.0
        };

        let progress = ExtractionProgress {
            partition_name: partition_name.to_string(),
            current_operation: current_op,
            total_operations: total_ops,
            percentage,
            status: ExtractionStatus::InProgress,
        };

        // call without holding any lock
        let should_continue = (self.callback)(progress);

        if !should_continue {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }

    fn on_complete(&self, partition_name: &str, total_operations: u64) {
        let progress = ExtractionProgress {
            partition_name: partition_name.to_string(),
            current_operation: total_operations,
            total_operations,
            percentage: 100.0,
            status: ExtractionStatus::Completed,
        };

        // call without holding any lock
        (self.callback)(progress);
    }

    fn on_warning(&self, partition_name: &str, operation_index: usize, message: String) {
        let progress = ExtractionProgress {
            partition_name: partition_name.to_string(),
            current_operation: operation_index as u64,
            total_operations: 0,
            percentage: 0.0,
            status: ExtractionStatus::Warning {
                operation_index,
                message,
            },
        };

        // call without holding any lock
        (self.callback)(progress);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/* Extract Partition (Payload.bin) */

/// extract a single partition from a payload.bin file
///
/// this function extracts a single partition. For parallel extraction of multiple
/// partitions, call this function from multiple threads (one per partition).
/// the caller is responsible for managing parallelization and thread limits.
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
/// let handles: Vec<_> = vec!["system", "vendor", "boot"]
///     .into_iter()
///     .map(|partition| {
///         thread::spawn(move || {
///             extract_partition("payload.bin", partition, format!("out/{}", partition), None)
///         })
///     })
///     .collect();
///
/// for handle in handles {
///     handle.join().unwrap().unwrap();
/// }
/// ```
pub fn extract_partition<P1, P2>(
    payload_path: P1,
    partition_name: &str,
    output_path: P2,
    progress_callback: Option<ProgressCallback>,
) -> Result<()>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    // check if we're already inside a tokio runtime
    if tokio::runtime::Handle::try_current().is_ok() {
        panic!(
            "extract_partition cannot be called from within an async runtime. Use spawn_blocking or call from a synchronous context."
        );
    }

    RUNTIME.block_on(async {
        let (manifest, data_offset) = parse_local_payload(payload_path.as_ref()).await?;

        let partition = manifest
            .partitions
            .iter()
            .find(|p| p.partition_name == partition_name)
            .ok_or_else(|| anyhow!("Partition '{}' not found in payload", partition_name))?;

        let block_size = manifest.block_size.unwrap_or(4096) as u64;

        if let Some(parent) = output_path.as_ref().parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let payload_reader =
            LocalAsyncPayloadReader::new(payload_path.as_ref().to_path_buf()).await?;

        let reporter: Box<dyn ProgressReporter> = if let Some(callback) = progress_callback {
            Box::new(CallbackProgressReporter::new(callback))
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

/* Extract Partition (ZIP file) */

/// extract a single partition from a ZIP file containing payload.bin
///
/// this function extracts a single partition. For parallel extraction of multiple
/// partitions, call this function from multiple threads (one per partition).
/// the caller is responsible for managing parallelization and thread limits.
///
/// # thread safety
/// This function can be safely called concurrently from multiple threads thanks
/// to the shared multi-threaded runtime.
///
/// # cancellation
/// If a progress callback is provided and returns `false`, the extraction will
/// be cancelled. note that cancellation may not be immediate - it depends on
/// when the next progress callback is triggered.
///
/// # Panics
/// panics if called from within an async runtime context (e.g. inside a tokio task).
/// this is a blocking function and must be called from a synchronous context only.
#[cfg(feature = "remote_zip")]
pub fn extract_partition_zip<P1, P2>(
    zip_path: P1,
    partition_name: &str,
    output_path: P2,
    progress_callback: Option<ProgressCallback>,
) -> Result<()>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    // check if we're already inside a tokio runtime
    if tokio::runtime::Handle::try_current().is_ok() {
        panic!(
            "extract_partition_zip cannot be called from within an async runtime. Use spawn_blocking or call from a synchronous context."
        );
    }

    RUNTIME.block_on(async {
        let (manifest, data_offset) =
            parse_local_zip_payload(zip_path.as_ref().to_path_buf()).await?;

        let partition = manifest
            .partitions
            .iter()
            .find(|p| p.partition_name == partition_name)
            .ok_or_else(|| anyhow!("Partition '{}' not found in payload", partition_name))?;

        let block_size = manifest.block_size.unwrap_or(4096) as u64;

        if let Some(parent) = output_path.as_ref().parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let payload_reader =
            LocalAsyncZipPayloadReader::new(zip_path.as_ref().to_path_buf()).await?;

        let reporter: Box<dyn ProgressReporter> = if let Some(callback) = progress_callback {
            Box::new(CallbackProgressReporter::new(callback))
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
