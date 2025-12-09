// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

#![allow(unused)]
use crate::cli::payload::file_detector::PayloadType;
use crate::cli::ui::ui_print::UiOutput;
use anyhow::Result;
use anyhow::anyhow;
use payload_dumper::DeltaArchiveManifest;
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
#[cfg(feature = "remote_zip")]
use payload_dumper::utils::format_size;
use std::path::Path;
use std::sync::Arc;

pub struct PayloadInfo {
    pub manifest: DeltaArchiveManifest,
    pub data_offset: u64,
    pub reader: Arc<dyn AsyncPayloadRead>,
}

/// loads and parses the payload, returns manifest, data offset, and reader
pub async fn load_payload(
    payload_path: &Path,
    payload_type: PayloadType,
    user_agent: Option<&str>,
    ui: &UiOutput,
) -> Result<PayloadInfo> {
    let payload_path_str = payload_path.to_string_lossy().to_string();

    let (manifest, data_offset) = match payload_type {
        PayloadType::RemoteZip => {
            #[cfg(feature = "remote_zip")]
            {
                ui.println("- Connecting to remote ZIP archive...");
                let (manifest, data_offset, content_length) =
                    parse_remote_payload(payload_path_str.clone(), user_agent).await?;
                ui.pb_eprintln(format!(
                    "- Remote ZIP size: {}",
                    format_size(content_length)
                ));
                (manifest, data_offset)
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
                    parse_remote_bin_payload(payload_path_str.clone(), user_agent).await?;
                ui.pb_eprintln(format!(
                    "- Remote .bin size: {}",
                    format_size(content_length)
                ));
                (manifest, data_offset)
            }
            #[cfg(not(feature = "remote_zip"))]
            {
                return Err(anyhow!("Remote .bin requires 'remote_zip' feature"));
            }
        }
        PayloadType::LocalZip => {
            #[cfg(feature = "local_zip")]
            {
                parse_local_zip_payload(payload_path.to_path_buf()).await?
            }
            #[cfg(not(feature = "local_zip"))]
            {
                return Err(anyhow!("Local ZIP requires 'local_zip' feature"));
            }
        }
        PayloadType::LocalBin => parse_local_payload(payload_path).await?,
    };

    // Create the appropriate reader
    let reader: Arc<dyn AsyncPayloadRead> = match payload_type {
        PayloadType::RemoteZip => {
            #[cfg(feature = "remote_zip")]
            {
                ui.println("- Preparing remote ZIP extraction...");
                Arc::new(
                    RemoteAsyncZipPayloadReader::new(payload_path_str.clone(), user_agent).await?,
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
                    RemoteAsyncBinPayloadReader::new(payload_path_str.clone(), user_agent).await?,
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
    })
}
