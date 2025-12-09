// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use anyhow::{Result, anyhow};
use payload_dumper::utils::{FileType, detect_file};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

#[derive(Debug, Clone, Copy)]
pub enum PayloadType {
    LocalZip,
    LocalBin,
    RemoteZip,
    RemoteBin,
}

/// reads magic bytes from a local file
async fn read_local_magic_bytes(path: &Path) -> Result<[u8; 4]> {
    let mut file = File::open(path).await?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).await?;
    Ok(magic)
}

/// reads magic bytes from a remote URL
#[cfg(feature = "remote_zip")]
async fn read_remote_magic_bytes(url: &str, user_agent: Option<&str>) -> Result<[u8; 4]> {
    use anyhow::Context;
    use payload_dumper::http::HttpReader;

    let http_reader = HttpReader::new(url.to_string(), user_agent)
        .await
        .context("Failed to initialize HTTP reader")?;

    let mut magic = [0u8; 4];
    http_reader
        .read_at(0, &mut magic)
        .await
        .context("Failed to read magic bytes from remote file")?;

    Ok(magic)
}

/// detects the payload file type (local/remote, zip/bin)
pub async fn detect_payload_type(
    payload_path: &Path,
    #[cfg(feature = "remote_zip")] user_agent: Option<&str>,
    #[cfg(not(feature = "remote_zip"))] _user_agent: Option<&str>,
) -> Result<PayloadType> {
    let payload_path_str = payload_path.to_string_lossy().to_string();
    let is_url =
        payload_path_str.starts_with("http://") || payload_path_str.starts_with("https://");

    let extension = payload_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let mut is_zip = extension == "zip";
    let mut is_bin = extension == "bin" || extension.is_empty();

    // check magic bytes if extension is unclear
    if !is_url && (!is_zip && !is_bin || extension.is_empty()) {
        let magic = read_local_magic_bytes(payload_path).await?;
        match detect_file(&magic) {
            Ok(FileType::Zip) => {
                is_zip = true;
                is_bin = false;
            }
            Ok(FileType::Bin) => {
                is_bin = true;
                is_zip = false;
            }
            Err(e) => {
                return Err(anyhow!(
                    "Unable to detect file type for {:?}. Extension: '{}', Error: {}",
                    payload_path.file_name().unwrap_or_default(),
                    extension,
                    e
                ));
            }
        }
    }

    // check magic bytes remotely
    if is_url && extension.is_empty() {
        #[cfg(feature = "remote_zip")]
        {
            let magic = read_remote_magic_bytes(&payload_path_str, user_agent).await?;
            match detect_file(&magic) {
                Ok(FileType::Zip) => {
                    is_zip = true;
                    is_bin = false;
                }
                Ok(FileType::Bin) => {
                    is_bin = true;
                    is_zip = false;
                }
                Err(e) => {
                    return Err(anyhow!(
                        "Unable to detect remote file type for {}: {}",
                        payload_path_str,
                        e
                    ));
                }
            }
        }
        #[cfg(not(feature = "remote_zip"))]
        {
            return Err(anyhow!(
                "Remote file type detection requires the 'remote_zip' feature"
            ));
        }
    }

    // validate we have a supported file type
    if !is_zip && !is_bin {
        return Err(anyhow!(
            "Unsupported file type. Only .bin and .zip files are supported"
        ));
    }

    // validate feature requirements
    if is_url && is_zip {
        #[cfg(not(feature = "remote_zip"))]
        return Err(anyhow!(
            "Remote ZIP processing requires the 'remote_zip' feature. \
             Please recompile with --features remote_zip"
        ));
    }

    if is_zip && !is_url {
        #[cfg(not(feature = "local_zip"))]
        return Err(anyhow!(
            "Local ZIP processing requires the 'local_zip' feature. \
             Please recompile with --features local_zip"
        ));
    }

    // determine payload type
    let payload_type = match (is_url, is_zip) {
        (false, true) => PayloadType::LocalZip,
        (false, false) => PayloadType::LocalBin,
        (true, true) => PayloadType::RemoteZip,
        (true, false) => PayloadType::RemoteBin,
    };

    Ok(payload_type)
}
