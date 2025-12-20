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


async fn read_sn(path: &Path) -> Result<[u8; 4]> {
    let mut file = File::open(path).await?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).await?;
    Ok(magic)
}

/// reads magic bytes from a remote URL
#[cfg(feature = "remote_zip")]
async fn read_r_sn(
    url: &str,
    user_agent: Option<&str>,
    cookies: Option<&str>,
) -> Result<[u8; 4]> {
    use anyhow::Context;
    use payload_dumper::http::HttpReader;

    let http_reader = HttpReader::new(url.to_string(), user_agent, cookies)
        .await
        .context("Failed to initialize HTTP reader")?;

    let mut magic = [0u8; 4];
    http_reader
        .read_at(0, &mut magic)
        .await
        .context("Failed to read magic bytes from remote file")?;

    Ok(magic)
}

/// detects the payload file type (local/remote, zip/bin) by checking magic bytes
/// 
/// this function always validates files using magic bytes rather than relying on
/// file extensions, providing reliable file type detection even for files with
/// incorrect or missing extensions.
pub async fn detect_payload_type(
    payload_path: &Path,
    user_agent: Option<&str>,
    cookies: Option<&str>,
) -> Result<PayloadType> {
    let payload_path_str = payload_path.to_string_lossy().to_string();
    let is_url = payload_path_str.starts_with("http://") || payload_path_str.starts_with("https://");

    // always check magic bytes, it's fast for local files and reliable for all cases
    let file_type = if is_url {
        #[cfg(feature = "remote_zip")]
        {
            let magic = read_r_sn(&payload_path_str, user_agent, cookies).await?;
            detect_file(&magic).map_err(|e| {
                anyhow!(
                    "Unable to detect remote file type for {}: {}",
                    payload_path_str,
                    e
                )
            })?
        }
        #[cfg(not(feature = "remote_zip"))]
        {
            return Err(anyhow!(
                "Remote file processing requires the 'remote_zip' feature. \
                 Please recompile with --features remote_zip"
            ));
        }
    } else {
        let magic = read_sn(payload_path).await?;
        detect_file(&magic).map_err(|e| {
            anyhow!(
                "Unable to detect file type for {:?}: {}. Only .bin and .zip files are supported",
                payload_path.file_name().unwrap_or_default(),
                e
            )
        })?
    };

    // validate feature requirements and return appropriate payload type
    match (is_url, file_type) {
        (false, FileType::Zip) => {
            #[cfg(not(feature = "local_zip"))]
            return Err(anyhow!(
                "Local ZIP processing requires the 'local_zip' feature. \
                 Please recompile with --features local_zip"
            ));
            
            #[cfg(feature = "local_zip")]
            Ok(PayloadType::LocalZip)
        }
        (false, FileType::Bin) => Ok(PayloadType::LocalBin),
        (true, FileType::Zip) => {
            #[cfg(not(feature = "remote_zip"))]
            return Err(anyhow!(
                "Remote ZIP processing requires the 'remote_zip' feature. \
                 Please recompile with --features remote_zip"
            ));
            
            #[cfg(feature = "remote_zip")]
            Ok(PayloadType::RemoteZip)
        }
        (true, FileType::Bin) => {
            #[cfg(not(feature = "remote_zip"))]
            return Err(anyhow!(
                "Remote BIN processing requires the 'remote_zip' feature. \
                 Please recompile with --features remote_zip"
            ));
            
            #[cfg(feature = "remote_zip")]
            Ok(PayloadType::RemoteBin)
        }
    }
}