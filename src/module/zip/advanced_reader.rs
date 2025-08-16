use std::io::{self, Read, Seek, SeekFrom};

/// Advanced reader trait that supports both traditional I/O and memory-mapped operations
pub trait AdvancedReader: Read + Seek + Send {
    /// Read data at a specific position without changing the current seek position
    fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize>;

    /// Get a direct slice of memory if available (only works with memory-mapped readers)
    fn get_slice(&self, pos: u64, len: usize) -> Option<&[u8]>;

    /// Get the total size of the readable data
    fn size(&self) -> io::Result<u64>;

    /// Check if this reader supports zero-copy operations
    fn supports_zero_copy(&self) -> bool {
        false
    }
}

/// Memory-mapped file reader for optimal performance
pub struct MmapReader {
    mmap: memmap2::Mmap,
    position: u64,
}

impl MmapReader {
    pub fn new(mmap: memmap2::Mmap) -> Self {
        Self { mmap, position: 0 }
    }

    pub fn from_file(file: std::fs::File) -> io::Result<Self> {
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Ok(Self::new(mmap))
    }
}

impl Read for MmapReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let start = self.position as usize;
        if start >= self.mmap.len() {
            return Ok(0); // EOF
        }

        let end = std::cmp::min(start + buf.len(), self.mmap.len());
        let bytes_to_read = end - start;

        buf[..bytes_to_read].copy_from_slice(&self.mmap[start..end]);
        self.position += bytes_to_read as u64;

        Ok(bytes_to_read)
    }
}

impl Seek for MmapReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                if offset >= 0 {
                    self.position.saturating_add(offset as u64)
                } else {
                    self.position.saturating_sub(offset.abs() as u64)
                }
            }
            SeekFrom::End(offset) => {
                let file_size = self.mmap.len() as u64;
                if offset >= 0 {
                    file_size.saturating_add(offset as u64)
                } else {
                    file_size.saturating_sub(offset.abs() as u64)
                }
            }
        };

        if new_pos > self.mmap.len() as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Attempted to seek past end of file",
            ));
        }

        self.position = new_pos;
        Ok(self.position)
    }
}

impl AdvancedReader for MmapReader {
    fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        let start = pos as usize;
        if start >= self.mmap.len() {
            return Ok(0);
        }

        let end = std::cmp::min(start + buf.len(), self.mmap.len());
        let bytes_to_read = end - start;

        if bytes_to_read > 0 {
            buf[..bytes_to_read].copy_from_slice(&self.mmap[start..end]);
        }

        Ok(bytes_to_read)
    }

    fn get_slice(&self, pos: u64, len: usize) -> Option<&[u8]> {
        let start = pos as usize;
        let end = start.saturating_add(len);

        if start < self.mmap.len() && end <= self.mmap.len() {
            Some(&self.mmap[start..end])
        } else {
            None
        }
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.mmap.len() as u64)
    }

    fn supports_zero_copy(&self) -> bool {
        true
    }
}

/// Wrapper for traditional File I/O to implement AdvancedReader
pub struct FileReader {
    file: std::fs::File,
    size: u64,
}

impl FileReader {
    pub fn new(mut file: std::fs::File) -> io::Result<Self> {
        let size = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self { file, size })
    }
}

impl Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for FileReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl AdvancedReader for FileReader {
    fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.file.seek(SeekFrom::Start(pos))?;
        self.file.read(buf)
    }

    fn get_slice(&self, _pos: u64, _len: usize) -> Option<&[u8]> {
        None // Traditional files don't support direct memory access
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.size)
    }

    fn supports_zero_copy(&self) -> bool {
        false
    }
}

/// Smart factory for creating the optimal reader based on file size and capabilities
pub fn create_optimal_reader(path: &std::path::Path) -> io::Result<Box<dyn AdvancedReader>> {
    let file = std::fs::File::open(path)?;
    let file_size = file.metadata()?.len();

    // Use memory mapping for files larger than 10MB
    if file_size > 10 * 1024 * 1024 {
        match MmapReader::from_file(file) {
            Ok(mmap_reader) => {
                // eprintln!(
                //     "[DEBUG] Using memory-mapped reading for {:.1}GB file",
                //     file_size as f64 / (1024.0 * 1024.0 * 1024.0)
                // );
                return Ok(Box::new(mmap_reader));
            }
            Err(e) => {
                eprintln!(
                    "Memory mapping failed, falling back to traditional I/O: {}",
                    e
                );
                let file = std::fs::File::open(path)?;
                return Ok(Box::new(FileReader::new(file)?));
            }
        }
    }

    // For smaller files, traditional I/O is fine
    Ok(Box::new(FileReader::new(file)?))
}
