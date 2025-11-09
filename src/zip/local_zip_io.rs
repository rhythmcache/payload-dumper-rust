use crate::zip::zip_io::ZipIO;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Mutex;

pub struct LocalZipIO {
    file: Arc<Mutex<File>>,
    size: u64,
}

impl LocalZipIO {
    pub async fn new(path: PathBuf) -> Result<Self> {
        let file = File::open(&path).await?;
        let size = file.metadata().await?.len();

        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            size,
        })
    }
}

#[async_trait]
impl ZipIO for LocalZipIO {
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let mut file = self.file.lock().await;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        file.read_exact(buf).await?;
        Ok(())
    }

    async fn size(&self) -> Result<u64> {
        Ok(self.size)
    }
}
