// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use crate::cli::ui::ui_print::ExtractionProgress;

#[cfg(feature = "prefetch")]
use crate::cli::ui::ui_print::DownloadProgress;

///  reporter for extraction progress
pub struct CliExtractionReporter {
    progress: ExtractionProgress,
}

impl CliExtractionReporter {
    pub fn new(progress: ExtractionProgress) -> Self {
        Self { progress }
    }
}

// this implements the ProgressReporter trait from payload_dumper
// the trait itself lives in the library, but this implementation
// lives in this cli layer and uses cli-specific ui components
impl payload_dumper::payload::payload_dumper::ProgressReporter for CliExtractionReporter {
    fn on_start(&self, partition_name: &str, _total_operations: u64) {
        self.progress.set_message(partition_name.to_string());
        self.progress.set_position(0);
    }

    fn on_progress(&self, _partition_name: &str, current_op: u64, total_ops: u64) {
        let percentage = (current_op as f64 / total_ops as f64 * 100.0) as u64;
        self.progress.set_position(percentage);
    }

    fn on_complete(&self, partition_name: &str, total_operations: u64) {
        self.progress
            .finish_with_message(format!("✓ {} ({} ops)", partition_name, total_operations));
    }

    fn on_warning(&self, partition_name: &str, operation_index: usize, message: String) {
        // warnings are printed directly to stderr
        eprintln!(
            "  Warning [{}:op{}]: {}",
            partition_name, operation_index, message
        );
    }
}

/// cli reporter for download progress
/// uses the UI layer's DownloadProgress wrapper
#[cfg(feature = "prefetch")]
pub struct CliDownloadReporter {
    progress: DownloadProgress,
}

#[cfg(feature = "prefetch")]
impl CliDownloadReporter {
    pub fn new(progress: DownloadProgress) -> Self {
        Self { progress }
    }
}

// this implements the DownloadProgressReporter trait from prefetch module
// the trait itself lives in the library, but this implementation
// lives in the cli layer and uses cli-specific ui components
#[cfg(feature = "prefetch")]
impl payload_dumper::prefetch::DownloadProgressReporter for CliDownloadReporter {
    fn on_download_start(&self, partition_name: &str, total_bytes: u64) {
        use payload_dumper::utils::format_size;
        self.progress.set_message(format!(
            "Downloading {} [0 bytes/{}]",
            partition_name,
            format_size(total_bytes)
        ));
        self.progress.set_position(0);
    }

    fn on_download_progress(&self, partition_name: &str, downloaded: u64, total: u64) {
        use payload_dumper::utils::format_size;
        let percent = (downloaded as f64 / total as f64 * 100.0) as u64;

        self.progress.set_message(format!(
            "Downloading {} [{}/{}]",
            partition_name,
            format_size(downloaded),
            format_size(total)
        ));
        self.progress.set_position(percent);
    }

    fn on_download_complete(&self, partition_name: &str, total_bytes: u64) {
        use payload_dumper::utils::format_size;
        self.progress.finish_with_message(format!(
            "Downloaded {} [{}]",
            partition_name,
            format_size(total_bytes)
        ));
    }
}
