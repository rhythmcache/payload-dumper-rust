use crate::payload_dumper::AsyncPayloadRead;
use crate::zip::local_zip_io::LocalZipIO;
use crate::zip::zip::ZipParser;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncSeek;
use tokio::io::{AsyncRead, AsyncSeekExt, BufReader};
use tokio::sync::Semaphore;

/// a seekable reader for payload.bin within a ZIP file
pub struct ZipPayloadFile {
    file: File,
    payload_offset: u64,
    payload_size: u64,
    position: u64,
}

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

pub struct LocalAsyncZipPayloadReader {
    path: PathBuf,
    payload_offset: u64,
    payload_size: u64,
    semaphore: Arc<Semaphore>,
}

impl LocalAsyncZipPayloadReader {
    pub async fn new(zip_path: PathBuf) -> Result<Self> {
        // use LocalZipIO for parsing
        let io = LocalZipIO::new(zip_path.clone()).await?;

        // find payload.bin entry
        let entry = ZipParser::find_payload_entry(&io).await?;

        // get actual data offset (after local header)
        let data_offset = ZipParser::get_data_offset(&io, &entry).await?;

        // verify it's actually a payload file
        ZipParser::verify_payload_magic(&io, data_offset).await?;

        let max_concurrent_reads = num_cpus::get() * 2;

        Ok(Self {
            path: zip_path,
            payload_offset: data_offset,
            payload_size: entry.uncompressed_size,
            semaphore: Arc::new(Semaphore::new(max_concurrent_reads)),
        })
    }

    /*  pub fn payload_offset(&self) -> u64 {
        self.payload_offset
    }

    pub fn payload_size(&self) -> u64 {
        self.payload_size
    } */
}

#[async_trait]
impl AsyncPayloadRead for LocalAsyncZipPayloadReader {
    async fn stream_from(
        &self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send>>> {
        // offset is relative to payload.bin start, add ZIP offset
        let absolute_offset = self.payload_offset + offset;

        let permit = self.semaphore.clone().acquire_owned().await?;
        let mut file = File::open(&self.path).await?;
        file.seek(std::io::SeekFrom::Start(absolute_offset)).await?;

        Ok(Box::pin(LimitedReader {
            inner: BufReader::new(file),
            remaining: length,
            _permit: permit,
        }))
    }
}

struct LimitedReader<R: AsyncRead + Unpin> {
    inner: R,
    remaining: u64,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl<R: AsyncRead + Unpin> AsyncRead for LimitedReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.remaining == 0 {
            return std::task::Poll::Ready(Ok(()));
        }

        let max_read = std::cmp::min(buf.remaining() as u64, self.remaining) as usize;
        let mut limited_buf = buf.take(max_read);

        let pin = Pin::new(&mut self.inner);
        match pin.poll_read(cx, &mut limited_buf) {
            std::task::Poll::Ready(Ok(())) => {
                let filled = limited_buf.filled().len();
                self.remaining -= filled as u64;
                buf.advance(filled);
                std::task::Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}
