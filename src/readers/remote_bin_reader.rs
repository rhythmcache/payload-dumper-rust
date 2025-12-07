/*
 * Note: It is highly unlikely that a raw payload.bin file will ever be served
 * directly by any normal HTTP server unless it is a custom implementation.
 * Still, we include support for this to maintain compatibility and consistency.
 */

use crate::http::HttpReader;
use crate::payload::payload_dumper::{AsyncPayloadRead, PayloadReader};
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncRead;

/// async payload reader for remote .bin files (not in ZIP)
pub struct RemoteAsyncBinPayloadReader {
    pub http_reader: Arc<HttpReader>,
}

impl RemoteAsyncBinPayloadReader {
    pub async fn new(url: String, user_agent: Option<&str>) -> Result<Self> {
        let http_reader = HttpReader::new(url, user_agent).await?;

        Ok(Self {
            http_reader: Arc::new(http_reader),
        })
    }
}

#[async_trait]
impl AsyncPayloadRead for RemoteAsyncBinPayloadReader {
    async fn open_reader(&self) -> Result<Box<dyn PayloadReader>> {
        Ok(Box::new(RemoteBinPayloadReader {
            http_reader: Arc::clone(&self.http_reader),
        }))
    }
}

struct RemoteBinPayloadReader {
    http_reader: Arc<HttpReader>,
}

#[async_trait]
impl PayloadReader for RemoteBinPayloadReader {
    async fn read_range(
        &mut self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send + '_>>> {
        let end = offset + length - 1;
        let range = format!("bytes={}-{}", offset, end);

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

        let stream = response.bytes_stream();
        let reader = tokio_util::io::StreamReader::new(
            stream.map(|result| result.map_err(std::io::Error::other)),
        );

        Ok(Box::pin(reader))
    }
}
