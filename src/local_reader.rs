use crate::payload_dumper::AsyncPayloadRead;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncSeekExt;
use tokio::io::{AsyncRead, BufReader};
use tokio::sync::Semaphore;

pub struct LocalAsyncPayloadReader {
    path: PathBuf,
    semaphore: Arc<Semaphore>,
}

impl LocalAsyncPayloadReader {
    pub async fn new(path: PathBuf) -> Result<Self> {
        File::open(&path).await?;
        let max_concurrent_reads = num_cpus::get() * 2;
        Ok(Self {
            path,
            semaphore: Arc::new(Semaphore::new(max_concurrent_reads)),
        })
    }
}

#[async_trait]
impl AsyncPayloadRead for LocalAsyncPayloadReader {
    async fn stream_from(
        &self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send>>> {
        let permit = self.semaphore.clone().acquire_owned().await?;
        let mut file = File::open(&self.path).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;

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
