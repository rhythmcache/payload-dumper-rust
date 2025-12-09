// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use crate::constants::{PAYLOAD_MAGIC, ZIP_MAGIC};
use anyhow::{Result, anyhow};
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum FileType {
    Zip,
    Bin,
}

/// detects file type from magic bytes
pub fn detect_file(magic: &[u8; 4]) -> Result<FileType> {
    if magic.starts_with(&ZIP_MAGIC) {
        return Ok(FileType::Zip);
    }

    if magic == PAYLOAD_MAGIC {
        return Ok(FileType::Bin);
    }

    Err(anyhow!(
        "Magic mismatch: got {:02X?}, expected 'PK' or 'CrAU'",
        magic
    ))
}

pub fn format_elapsed_time(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let millis = duration.subsec_millis();

    if hours > 0 {
        format!("{}h {}m {}.{:03}s", hours, mins, secs, millis)
    } else if mins > 0 {
        format!("{}m {}.{:03}s", mins, secs, millis)
    } else {
        format!("{}.{:03}s", secs, millis)
    }
}

pub fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.2} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.2} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.2} KB", size as f64 / KB as f64)
    } else {
        format!("{} bytes", size)
    }
}
