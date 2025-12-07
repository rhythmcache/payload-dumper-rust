// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust
//
// This file is part of payload-dumper-rust. It implements components used for
// extracting and processing Android OTA payloads.

use crate::http::HttpReader;
use crate::payload::payload_dumper::{AsyncPayloadRead, PayloadReader};
use crate::zip::zip::ZipParser;
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncRead;

/// async payload reader for remote ZIP files
pub struct RemoteAsyncZipPayloadReader {
    pub http_reader: Arc<HttpReader>,
    payload_offset: u64,
    payload_size: u64,
}

impl RemoteAsyncZipPayloadReader {
    pub async fn new(url: String, user_agent: Option<&str>) -> Result<Self> {
        let http_reader = HttpReader::new(url, user_agent).await?;

        let entry = ZipParser::find_payload_entry(&http_reader).await?;
        let payload_offset = ZipParser::get_data_offset(&http_reader, &entry).await?;
        ZipParser::verify_payload_magic(&http_reader, payload_offset).await?;

        Ok(Self {
            http_reader: Arc::new(http_reader),
            payload_offset,
            payload_size: entry.uncompressed_size,
        })
    }
}

#[async_trait]
impl AsyncPayloadRead for RemoteAsyncZipPayloadReader {
    async fn open_reader(&self) -> Result<Box<dyn PayloadReader>> {
        Ok(Box::new(RemotePayloadReader {
            http_reader: Arc::clone(&self.http_reader),
            payload_offset: self.payload_offset,
            payload_size: self.payload_size,
        }))
    }
}

struct RemotePayloadReader {
    http_reader: Arc<HttpReader>,
    payload_offset: u64,
    payload_size: u64,
}

#[async_trait]
impl PayloadReader for RemotePayloadReader {
    async fn read_range(
        &mut self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send + '_>>> {
        let absolute_offset = self.payload_offset + offset;

        if absolute_offset + length > self.payload_offset + self.payload_size {
            return Err(anyhow::anyhow!(
                "Read request exceeds payload bounds: offset={}, length={}, payload_size={}",
                offset,
                length,
                self.payload_size
            ));
        }

        let end = absolute_offset + length - 1;
        let range = format!("bytes={}-{}", absolute_offset, end);

        let response = self
            .http_reader
            .client
            .get(&self.http_reader.url)
            .header(reqwest::header::RANGE, range)
            .send()
            .await?;

        if !response.status().is_success() && response.status().as_u16() != 206 {
            return Err(anyhow::anyhow!(
                "Range request failed: {}",
                response.status()
            ));
        }

        // convert response to AsyncRead stream
        let stream = response.bytes_stream();
        let reader = tokio_util::io::StreamReader::new(
            stream.map(|result| result.map_err(std::io::Error::other)),
        );

        Ok(Box::pin(reader))
    }
}
