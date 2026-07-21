// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust
use ahash::AHashSet as HashSet;
use anyhow::{Result, anyhow};
use payload_dumper::metadata::get_metadata;
use payload_dumper::structs::DeltaArchiveManifest;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;
/// handles metadata extraction and saving based on the provided mode
///
/// # arguments
/// * `manifest` - the parsed payload manifest
/// * `out_dir` - output directory path (or a direct file path ending in .json)
/// * `data_offset` - offset where payload data starts
/// * `mode` - metadata mode: "compact" or "full"
/// * `filter_images` - comma-separated partition names to filter, or empty for all
/// * `is_stdout` - whether output is directed to stdout
pub async fn handle_metadata_extraction(
    manifest: &DeltaArchiveManifest,
    out_dir: &Path,
    data_offset: u64,
    mode: &str,
    filter_images: &str,
    is_stdout: bool,
) -> Result<()> {
    let full_mode = mode == "full";
    // parse filter partitions if provided
    let filter_partitions: Option<HashSet<&str>> = if !filter_images.is_empty() {
        Some(filter_images.split(',').collect())
    } else {
        None
    };
    // generate metadata
    let metadata =
        get_metadata(manifest, data_offset, full_mode, filter_partitions.as_ref()).await?;
    // serialize to JSON
    let json_output = serde_json::to_string_pretty(&metadata)
        .map_err(|e| anyhow!("Failed to serialize metadata: {}", e))?;
    // output handling
    if is_stdout {
        // write to stdout
        let mut stdout = tokio::io::stdout();
        stdout
            .write_all(json_output.as_bytes())
            .await
            .map_err(|e| anyhow!("Failed to write metadata to stdout: {}", e))?;
        stdout
            .flush()
            .await
            .map_err(|e| anyhow!("Failed to flush stdout: {}", e))?;
    } else {
        // if out_dir looks like a target file (has an extension), write directly to it.
        // otherwise treat it as a directory and save metadata.json inside it.
        let metadata_file = if out_dir.extension().is_some() {
            out_dir.to_path_buf()
        } else {
            out_dir.join("metadata.json")
        };

        if let Some(parent) = metadata_file.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| anyhow!("Failed to create output directory: {}", e))?;
        }

        fs::write(&metadata_file, json_output)
            .await
            .map_err(|e| anyhow!("Failed to write metadata file: {}", e))?;

        println!("- Metadata saved to: {}", metadata_file.display());
    }
    Ok(())
}
