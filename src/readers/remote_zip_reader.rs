use crate::http::HttpReader;
use crate::payload::payload_dumper::AsyncPayloadRead;
use crate::zip::zip::ZipParser;
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use std::pin::Pin;
use tokio::io::AsyncRead;

/// async payload reader for remote ZIP files
pub struct RemoteAsyncZipPayloadReader {
    http_reader: HttpReader,
    payload_offset: u64,
    payload_size: u64,
    streaming_client: reqwest::Client,
}

impl RemoteAsyncZipPayloadReader {
    pub async fn new(url: String, user_agent: Option<&str>) -> Result<Self> {
        let http_reader = HttpReader::new(url, user_agent).await?;

        let entry = ZipParser::find_payload_entry(&http_reader).await?;
        let payload_offset = ZipParser::get_data_offset(&http_reader, &entry).await?;
        ZipParser::verify_payload_magic(&http_reader, payload_offset).await?;

        let streaming_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .pool_max_idle_per_host(10)
            .build()?;

        Ok(Self {
            http_reader,
            payload_offset,
            payload_size: entry.uncompressed_size,
            streaming_client,
        })
    }
}

#[async_trait]
impl AsyncPayloadRead for RemoteAsyncZipPayloadReader {
    async fn stream_from(
        &self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send>>> {
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
            .streaming_client
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
