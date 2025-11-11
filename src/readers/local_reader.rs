use crate::payload::payload_dumper::{AsyncPayloadRead, PayloadReader};
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncSeekExt;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
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
    async fn open_reader(&self) -> Result<Box<dyn PayloadReader>> {
        let permit = self.semaphore.clone().acquire_owned().await?;
        let file = File::open(&self.path).await?;
        Ok(Box::new(LocalPayloadReader {
            file: BufReader::new(file),
            _permit: permit,
        }))
    }
}

struct LocalPayloadReader {
    file: BufReader<File>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[async_trait]
impl PayloadReader for LocalPayloadReader {
    async fn read_range(
        &mut self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send + '_>>> {
        self.file.seek(std::io::SeekFrom::Start(offset)).await?;
        Ok(Box::pin((&mut self.file).take(length)))
    }
}
