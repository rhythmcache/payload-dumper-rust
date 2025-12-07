// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust
//
// This file is part of payload-dumper-rust. It implements components used for
// extracting and processing Android OTA payloads.

use crate::DeltaArchiveManifest;
use crate::constants::{PAYLOAD_MAGIC, SUPPORTED_PAYLOAD_VERSION};
#[cfg(feature = "remote_zip")]
use crate::http::HttpReader;
#[cfg(feature = "local_zip")]
use crate::zip::local_zip_io::LocalZipIO;
#[cfg(feature = "local_zip")]
use crate::zip::zip::ZipParser;
use anyhow::{Result, anyhow};
use prost::Message;
#[cfg(feature = "local_zip")]
use std::path::PathBuf;
#[cfg(feature = "local_zip")]
use std::pin::Pin;
#[cfg(feature = "local_zip")]
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

/// parse payload from any async reader that supports seeking
/// returns (manifest, data_offset)
pub async fn parse_payload<R>(mut reader: R) -> Result<(DeltaArchiveManifest, u64)>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    reader.seek(std::io::SeekFrom::Start(0)).await?;

    // read and validate magic
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic).await?;
    if &magic != PAYLOAD_MAGIC {
        return Err(anyhow!("Invalid payload file: magic 'CrAU' not found"));
    }

    // read and validate version
    let version = reader.read_u64().await?;
    if version != SUPPORTED_PAYLOAD_VERSION {
        return Err(anyhow!("Unsupported payload version: {}", version));
    }

    // read sizes
    let manifest_size = reader.read_u64().await?;
    let metadata_signature_size = reader.read_u32().await?;

    // read manifest
    let mut manifest_bytes = vec![0u8; manifest_size as usize];
    reader.read_exact(&mut manifest_bytes).await?;

    // skip metadata signature
    reader
        .seek(std::io::SeekFrom::Current(metadata_signature_size as i64))
        .await?;

    // get data offset
    let data_offset = reader.stream_position().await?;

    // decode manifest
    let manifest = DeltaArchiveManifest::decode(&manifest_bytes[..])?;

    Ok((manifest, data_offset))
}

/// returns (manifest, data_offset)
#[cfg(feature = "remote_zip")]
pub async fn parse_remote_payload(
    url: String,
    user_agent: Option<&str>,
) -> Result<(DeltaArchiveManifest, u64, u64)> {
    // Added u64 for content_length
    let http_reader = HttpReader::new(url, user_agent).await?;

    // Get the content length to return
    let content_length = http_reader.content_length;

    let entry = ZipParser::find_payload_entry(&http_reader).await?;
    let payload_offset = ZipParser::get_data_offset(&http_reader, &entry).await?;
    ZipParser::verify_payload_magic(&http_reader, payload_offset).await?;

    // Now use your existing parse_remote_payload implementation
    let mut pos = payload_offset;

    // helper to read and advance position
    async fn read_at(http_reader: &HttpReader, pos: &mut u64, buf: &mut [u8]) -> Result<()> {
        http_reader.read_at(*pos, buf).await?;
        *pos += buf.len() as u64;
        Ok(())
    }

    // read and validate magic
    let mut magic = [0u8; 4];
    read_at(&http_reader, &mut pos, &mut magic).await?;
    if &magic != PAYLOAD_MAGIC {
        return Err(anyhow!("Invalid payload file: magic 'CrAU' not found"));
    }

    // read and validate version
    let mut buf = [0u8; 8];
    read_at(&http_reader, &mut pos, &mut buf).await?;
    let version = u64::from_be_bytes(buf);
    if version != SUPPORTED_PAYLOAD_VERSION {
        return Err(anyhow!("Unsupported payload version: {}", version));
    }

    // read manifest size
    read_at(&http_reader, &mut pos, &mut buf).await?;
    let manifest_size = u64::from_be_bytes(buf);

    // read metadata signature size
    let mut buf4 = [0u8; 4];
    read_at(&http_reader, &mut pos, &mut buf4).await?;
    let sig_size = u32::from_be_bytes(buf4);

    // read manifest
    let mut manifest_bytes = vec![0u8; manifest_size as usize];
    read_at(&http_reader, &mut pos, &mut manifest_bytes).await?;

    // skip signature, advance position
    pos += sig_size as u64;

    // data offset is relative to payload start
    let data_offset = pos - payload_offset;

    // decode manifest
    let manifest = DeltaArchiveManifest::decode(&manifest_bytes[..])?;

    Ok((manifest, data_offset, content_length)) // Return content_length too
}

/// parse payload from local file
pub async fn parse_local_payload(
    payload_path: &std::path::Path,
) -> Result<(DeltaArchiveManifest, u64)> {
    let file = tokio::fs::File::open(payload_path).await?;
    parse_payload(file).await
}

/// a seekable reader for payload.bin within a ZIP file
#[cfg(feature = "local_zip")]
pub struct ZipPayloadFile {
    file: File,
    payload_offset: u64,
    payload_size: u64,
    position: u64,
}

#[cfg(feature = "local_zip")]
impl ZipPayloadFile {
    pub async fn new(zip_path: PathBuf) -> Result<Self> {
        let io = LocalZipIO::new(zip_path.clone()).await?;
        let entry = ZipParser::find_payload_entry(&io).await?;
        let data_offset = ZipParser::get_data_offset(&io, &entry).await?;
        ZipParser::verify_payload_magic(&io, data_offset).await?;

        let file = File::open(&zip_path).await?;

        Ok(Self {
            file,
            payload_offset: data_offset,
            payload_size: entry.uncompressed_size,
            position: 0,
        })
    }
}

#[cfg(feature = "local_zip")]
impl AsyncRead for ZipPayloadFile {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let remaining = self.payload_size.saturating_sub(self.position);
        if remaining == 0 {
            return std::task::Poll::Ready(Ok(()));
        }

        let max_read = std::cmp::min(buf.remaining() as u64, remaining) as usize;
        let mut limited_buf = buf.take(max_read);

        let pin = Pin::new(&mut self.file);
        match pin.poll_read(cx, &mut limited_buf) {
            std::task::Poll::Ready(Ok(())) => {
                let filled = limited_buf.filled().len();
                self.position += filled as u64;
                buf.advance(filled);
                std::task::Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(feature = "local_zip")]
impl AsyncSeek for ZipPayloadFile {
    fn start_seek(mut self: Pin<&mut Self>, position: std::io::SeekFrom) -> std::io::Result<()> {
        let new_pos = match position {
            std::io::SeekFrom::Start(offset) => offset,
            std::io::SeekFrom::End(offset) => {
                if offset >= 0 {
                    self.payload_size.saturating_add(offset as u64)
                } else {
                    self.payload_size.saturating_sub((-offset) as u64)
                }
            }
            std::io::SeekFrom::Current(offset) => {
                if offset >= 0 {
                    self.position.saturating_add(offset as u64)
                } else {
                    self.position.saturating_sub((-offset) as u64)
                }
            }
        };

        if new_pos > self.payload_size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Seek beyond payload end",
            ));
        }

        self.position = new_pos;
        let absolute_pos = self.payload_offset + new_pos;
        Pin::new(&mut self.file).start_seek(std::io::SeekFrom::Start(absolute_pos))
    }

    fn poll_complete(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<u64>> {
        match Pin::new(&mut self.file).poll_complete(cx) {
            std::task::Poll::Ready(Ok(_)) => std::task::Poll::Ready(Ok(self.position)),
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e)),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// parse payload from local ZIP file
#[cfg(feature = "local_zip")]
pub async fn parse_local_zip_payload(zip_path: PathBuf) -> Result<(DeltaArchiveManifest, u64)> {
    let zip_payload = ZipPayloadFile::new(zip_path).await?;
    parse_payload(zip_payload).await
}

/// Parse payload from remote .bin file (not in ZIP)

#[cfg(feature = "local_zip")]
pub async fn parse_remote_bin_payload(
    url: String,
    user_agent: Option<&str>,
) -> Result<(DeltaArchiveManifest, u64, u64)> {
    #[cfg(feature = "remote_zip")]
    {
        let http_reader = HttpReader::new(url, user_agent).await?;
        let content_length = http_reader.content_length;

        let mut pos = 0u64;

        // Helper to read and advance position
        async fn read_at(http_reader: &HttpReader, pos: &mut u64, buf: &mut [u8]) -> Result<()> {
            http_reader.read_at(*pos, buf).await?;
            *pos += buf.len() as u64;
            Ok(())
        }

        // Read and validate magic
        let mut magic = [0u8; 4];
        read_at(&http_reader, &mut pos, &mut magic).await?;
        if &magic != PAYLOAD_MAGIC {
            return Err(anyhow!("Invalid payload file: magic 'CrAU' not found"));
        }

        // Read and validate version
        let mut buf = [0u8; 8];
        read_at(&http_reader, &mut pos, &mut buf).await?;
        let version = u64::from_be_bytes(buf);
        if version != SUPPORTED_PAYLOAD_VERSION {
            return Err(anyhow!("Unsupported payload version: {}", version));
        }

        // Read manifest size
        read_at(&http_reader, &mut pos, &mut buf).await?;
        let manifest_size = u64::from_be_bytes(buf);

        // Read metadata signature size
        let mut buf4 = [0u8; 4];
        read_at(&http_reader, &mut pos, &mut buf4).await?;
        let sig_size = u32::from_be_bytes(buf4);

        // Read manifest
        let mut manifest_bytes = vec![0u8; manifest_size as usize];
        read_at(&http_reader, &mut pos, &mut manifest_bytes).await?;

        // Skip signature
        pos += sig_size as u64;

        // Data offset is current position
        let data_offset = pos;

        // Decode manifest
        let manifest = DeltaArchiveManifest::decode(&manifest_bytes[..])?;

        Ok((manifest, data_offset, content_length))
    }
    #[cfg(not(feature = "remote_zip"))]
    {
        Err(anyhow!(
            "Remote .bin support requires the 'remote_zip' feature"
        ))
    }
}
