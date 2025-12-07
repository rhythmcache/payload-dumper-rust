use crate::zip::zip_io::ZipIO;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

pub struct LocalZipIO {
    file: std::fs::File,
    size: u64,
}

impl LocalZipIO {
    pub async fn new(path: PathBuf) -> Result<Self> {
        let file = std::fs::File::open(&path)?;
        let size = file.metadata()?.len();

        Ok(Self { file, size })
    }
}

#[async_trait]
impl ZipIO for LocalZipIO {
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        tokio::task::block_in_place(|| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileExt;
                self.file.read_exact_at(buf, offset)?;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::FileExt;
                self.file.seek_read(buf, offset)?;
            }
            Ok(())
        })
    }

    async fn size(&self) -> Result<u64> {
        Ok(self.size)
    }
}
