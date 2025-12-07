// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust
//
// This file is part of payload-dumper-rust. It implements components used for
// extracting and processing Android OTA payloads.

use crate::DeltaArchiveManifest;
use crate::constants::{PAYLOAD_MAGIC, ZIP_MAGIC};
use anyhow::{Result, anyhow};
use std::path::Path;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

#[derive(Debug, PartialEq)]
pub enum FileType {
    Zip,
    Bin,
}

pub async fn detect_file_type(path: &Path) -> Result<FileType> {
    let mut file = File::open(path).await?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).await?;

    if magic.starts_with(&ZIP_MAGIC) {
        return Ok(FileType::Zip);
    }

    if &magic == PAYLOAD_MAGIC {
        return Ok(FileType::Bin);
    }

    Err(anyhow!(
        "Magic mismatch in file {:?}: got {:02X?}, expected 'PK' or 'CrAU'",
        path.file_name().unwrap_or_default(),
        magic
    ))
}

#[cfg(feature = "remote_zip")]
pub async fn detect_remote_file_type(url: &str, user_agent: Option<&str>) -> Result<FileType> {
    use crate::http::HttpReader;
    use anyhow::Context;

    let http_reader = HttpReader::new(url.to_string(), user_agent)
        .await
        .context("Failed to initialize HTTP reader")?;

    let mut magic = [0u8; 4];
    http_reader
        .read_at(0, &mut magic)
        .await
        .context("Failed to read magic bytes from remote file")?;

    if magic.starts_with(&ZIP_MAGIC) {
        return Ok(FileType::Zip);
    }

    if &magic == PAYLOAD_MAGIC {
        return Ok(FileType::Bin);
    }

    Err(anyhow!(
        "Magic mismatch in remote file {}: got {:02X?}, expected 'PK' or 'CrAU'",
        url,
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

pub fn list_partitions(manifest: &DeltaArchiveManifest) {
    println!("{:<20} {:<15}", "Partition Name", "Size");
    println!("{}", "-".repeat(35));

    for partition in &manifest.partitions {
        let size = partition
            .new_partition_info
            .as_ref()
            .and_then(|info| info.size)
            .unwrap_or(0);

        println!(
            "{:<20} {:<15}",
            partition.partition_name,
            if size > 0 {
                format_size(size)
            } else {
                "Unknown".to_string()
            }
        );
    }
}
