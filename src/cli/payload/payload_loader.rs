// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

#![allow(unused)]
use crate::cli::payload::file_detector::PayloadType;
use crate::cli::ui::ui_print::UiOutput;
use anyhow::Result;
use anyhow::anyhow;
use payload_dumper::payload::payload_dumper::AsyncPayloadRead;
use payload_dumper::payload::payload_parser::parse_local_payload;
#[cfg(feature = "local_zip")]
use payload_dumper::payload::payload_parser::parse_local_zip_payload;
#[cfg(feature = "remote_zip")]
use payload_dumper::payload::payload_parser::{parse_remote_bin_payload, parse_remote_payload};
use payload_dumper::readers::local_reader::LocalAsyncPayloadReader;
#[cfg(feature = "local_zip")]
use payload_dumper::readers::local_zip_reader::LocalAsyncZipPayloadReader;
#[cfg(feature = "remote_zip")]
use payload_dumper::readers::remote_bin_reader::RemoteAsyncBinPayloadReader;
#[cfg(feature = "remote_zip")]
use payload_dumper::readers::remote_zip_reader::RemoteAsyncZipPayloadReader;
use payload_dumper::structs::{DeltaArchiveManifest, SourceInfo, ZipDetails};

use payload_dumper::utils::format_size;
#[cfg(any(feature = "local_zip", feature = "remote_zip"))]
use payload_dumper::zip::core_parser::ZipMetadataInfo;
use std::path::Path;
use std::sync::Arc;

pub struct PayloadInfo {
    pub manifest: DeltaArchiveManifest,
    pub data_offset: u64,
    pub reader: Arc<dyn AsyncPayloadRead>,
    pub source_info: Option<SourceInfo>,
}

#[cfg(any(feature = "local_zip", feature = "remote_zip"))]
fn create_zip_source_info(
    source_type: &str,
    path_or_url: &str,
    zip_info: &ZipMetadataInfo,
) -> SourceInfo {
    let file_name = Path::new(path_or_url)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path_or_url.to_string());

    let method_str = match zip_info.compression_method {
        0 => "STORED (0)".to_string(),
        8 => "DEFLATED (8)".to_string(),
        other => format!("UNKNOWN ({})", other),
    };

    let zip_details = ZipDetails {
        entry_name: zip_info.entry_name.clone(),
        header_offset: zip_info.header_offset,
        payload_data_offset: zip_info.payload_data_offset,
        uncompressed_size: zip_info.uncompressed_size,
        uncompressed_size_readable: format_size(zip_info.uncompressed_size),
        compressed_size: zip_info.compressed_size,
        compressed_size_readable: format_size(zip_info.compressed_size),
        compression_method: method_str,
        total_entries: zip_info.total_entries,
        central_directory_offset: zip_info.central_directory_offset,
    };

    SourceInfo {
        source_type: source_type.to_string(),
        file_name,
        file_path_or_url: path_or_url.to_string(),
        archive_size: Some(zip_info.archive_size),
        archive_size_readable: Some(format_size(zip_info.archive_size)),
        zip_details: Some(zip_details),
    }
}

/// loads and parses the payload, returns manifest, data offset, reader, and source info
pub async fn load_payload(
    payload_path: &Path,
    payload_type: PayloadType,
    user_agent: Option<&str>,
    cookies: Option<&str>,
    dns: Option<&str>,
    ui: &UiOutput,
) -> Result<PayloadInfo> {
    let payload_path_str = payload_path.to_string_lossy().to_string();

    let (manifest, data_offset, source_info) = match payload_type {
        PayloadType::RemoteZip => {
            #[cfg(feature = "remote_zip")]
            {
                ui.println("- Connecting to remote ZIP archive...");
                let (manifest, data_offset, zip_info) =
                    parse_remote_payload(payload_path_str.clone(), user_agent, cookies, dns)
                        .await?;
                ui.pb_eprintln(format!(
                    "- Remote ZIP size: {}",
                    format_size(zip_info.archive_size)
                ));
                let source_info =
                    create_zip_source_info("remote_zip", &payload_path_str, &zip_info);
                (manifest, data_offset, Some(source_info))
            }
            #[cfg(not(feature = "remote_zip"))]
            {
                return Err(anyhow!("Remote ZIP requires 'remote_zip' feature"));
            }
        }
        PayloadType::RemoteBin => {
            #[cfg(feature = "remote_zip")]
            {
                ui.println("- Connecting to remote .bin file...");
                let (manifest, data_offset, content_length) =
                    parse_remote_bin_payload(payload_path_str.clone(), user_agent, cookies, dns)
                        .await?;
                ui.pb_eprintln(format!(
                    "- Remote .bin size: {}",
                    format_size(content_length)
                ));
                let file_name = Path::new(&payload_path_str)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| payload_path_str.clone());

                let source_info = SourceInfo {
                    source_type: "remote_bin".to_string(),
                    file_name,
                    file_path_or_url: payload_path_str.clone(),
                    archive_size: Some(content_length),
                    archive_size_readable: Some(format_size(content_length)),
                    zip_details: None,
                };
                (manifest, data_offset, Some(source_info))
            }
            #[cfg(not(feature = "remote_zip"))]
            {
                return Err(anyhow!("Remote .bin requires 'remote_zip' feature"));
            }
        }
        PayloadType::LocalZip => {
            #[cfg(feature = "local_zip")]
            {
                let (manifest, data_offset, zip_info) =
                    parse_local_zip_payload(payload_path.to_path_buf()).await?;
                let source_info = create_zip_source_info("local_zip", &payload_path_str, &zip_info);
                (manifest, data_offset, Some(source_info))
            }
            #[cfg(not(feature = "local_zip"))]
            {
                return Err(anyhow!("Local ZIP requires 'local_zip' feature"));
            }
        }
        PayloadType::LocalBin => {
            let (manifest, data_offset) = parse_local_payload(payload_path).await?;
            let file_size = tokio::fs::metadata(payload_path)
                .await
                .map(|m| m.len())
                .ok();
            let file_name = payload_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| payload_path_str.clone());

            let source_info = SourceInfo {
                source_type: "local_bin".to_string(),
                file_name,
                file_path_or_url: payload_path_str.clone(),
                archive_size: file_size,
                archive_size_readable: file_size.map(format_size),
                zip_details: None,
            };
            (manifest, data_offset, Some(source_info))
        }
    };

    // Create the appropriate reader
    let reader: Arc<dyn AsyncPayloadRead> = match payload_type {
        PayloadType::RemoteZip => {
            #[cfg(feature = "remote_zip")]
            {
                ui.println("- Preparing remote ZIP extraction...");
                Arc::new(
                    RemoteAsyncZipPayloadReader::new(
                        payload_path_str.clone(),
                        user_agent,
                        cookies,
                        dns,
                    )
                    .await?,
                )
            }
            #[cfg(not(feature = "remote_zip"))]
            {
                return Err(anyhow!("Remote ZIP requires 'remote_zip' feature"));
            }
        }
        PayloadType::RemoteBin => {
            #[cfg(feature = "remote_zip")]
            {
                ui.println("- Preparing remote .bin extraction...");
                Arc::new(
                    RemoteAsyncBinPayloadReader::new(
                        payload_path_str.clone(),
                        user_agent,
                        cookies,
                        dns,
                    )
                    .await?,
                )
            }
            #[cfg(not(feature = "remote_zip"))]
            {
                return Err(anyhow!("Remote .bin requires 'remote_zip' feature"));
            }
        }
        PayloadType::LocalZip => {
            #[cfg(feature = "local_zip")]
            {
                Arc::new(LocalAsyncZipPayloadReader::new(payload_path.to_path_buf()).await?)
            }
            #[cfg(not(feature = "local_zip"))]
            {
                return Err(anyhow!("Local ZIP requires 'local_zip' feature"));
            }
        }
        PayloadType::LocalBin => {
            Arc::new(LocalAsyncPayloadReader::new(payload_path.to_path_buf()).await?)
        }
    };

    Ok(PayloadInfo {
        manifest,
        data_offset,
        reader,
        source_info,
    })
}
