use std::collections::HashMap;
use std::io::{Error, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use positioned_io::ReadAt;

use crate::module::zip::zip_core::{ZipEntry, ZipParser};

pub struct ZipDecoder<R: Read + Seek> {
    reader: R,
    entries: HashMap<String, ZipEntry>,
}

// Positioned I/O using Arc<File>
pub struct SharedZipPayloadReader {
    file: Arc<std::fs::File>,
    payload_entry: ZipEntry,
}

// sequential access
pub struct ZipPayloadReader<R: Read + Seek> {
    decoder: ZipDecoder<R>,
    current_entry: Option<ZipEntry>,
    current_position: u64,
}

pub type FileZipPayloadReader = ZipPayloadReader<std::fs::File>;

// positioned I/O for shared ZIP reader
impl SharedZipPayloadReader {
    pub fn new<P: AsRef<Path>>(path: P) -> IoResult<Self> {
        let mut file = std::fs::File::open(path.as_ref())?;

        // read ZIP metadata
        let entries = ZipDecoder::<std::fs::File>::read_central_directory(&mut file)?;

        let entry = entries
            .get("payload.bin")
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "payload.bin not found in ZIP"))?
            .clone();

        // get data offset
        let mut temp_file = std::fs::File::open(path.as_ref())?;
        let data_offset = ZipParser::get_data_offset(&mut temp_file, &entry)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;

        let mut payload_entry = entry;
        payload_entry.data_offset = data_offset;

        Ok(Self {
            file: Arc::new(std::fs::File::open(path)?),
            payload_entry,
        })
    }

    // read at a specific offset within payload.bin
    pub fn read_at_payload(&self, offset: u64, buf: &mut [u8]) -> IoResult<usize> {
        if offset >= self.payload_entry.uncompressed_size {
            return Ok(0);
        }

        let remaining = self.payload_entry.uncompressed_size - offset;
        let to_read = buf.len().min(remaining as usize);

        // absolute position in ZIP file
        let file_position = self.payload_entry.data_offset + offset;

        // use positioned read
        self.file.read_at(file_position, &mut buf[..to_read])
    }
}

// payloadRead trait for shared ZIP reader
impl crate::module::payload_dumper::PayloadRead for SharedZipPayloadReader {
    fn read_data_at(&self, offset: u64, buf: &mut [u8]) -> IoResult<()> {
        let mut total_read = 0;
        while total_read < buf.len() {
            let n = self.read_at_payload(offset + total_read as u64, &mut buf[total_read..])?;
            if n == 0 {
                return Err(Error::new(ErrorKind::UnexpectedEof, "Unexpected EOF"));
            }
            total_read += n;
        }
        Ok(())
    }
}

// sequential implementation
impl<R: Read + Seek> ZipPayloadReader<R> {
    pub fn new(reader: R) -> IoResult<Self> {
        let decoder = ZipDecoder::new(reader)?;
        Ok(ZipPayloadReader {
            decoder,
            current_entry: None,
            current_position: 0,
        })
    }

    pub fn load_payload_entry(&mut self) -> IoResult<()> {
        if let Some(entry) = self.decoder.get_entry("payload.bin") {
            let mut entry = entry.clone();
            let data_offset = self.decoder.get_data_offset(&entry)?;
            entry.data_offset = data_offset;

            self.current_entry = Some(entry);
            self.current_position = 0;
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::NotFound,
                "payload.bin not found in the zip",
            ))
        }
    }
}

impl FileZipPayloadReader {
    pub fn from_file<P: AsRef<Path>>(path: P) -> IoResult<Self> {
        let file = std::fs::File::open(path)?;
        let mut reader = Self::new(file)?;
        reader.load_payload_entry()?;
        Ok(reader)
    }
}

impl<R: Read + Seek> Read for ZipPayloadReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let entry = match &self.current_entry {
            Some(entry) => entry,
            None => {
                self.load_payload_entry()?;
                self.current_entry.as_ref().unwrap()
            }
        };

        if self.current_position >= entry.uncompressed_size {
            return Ok(0);
        }

        let remaining = entry.uncompressed_size - self.current_position;
        let to_read = buf.len().min(remaining as usize);

        let file_position = entry.data_offset + self.current_position;
        self.decoder.reader.seek(SeekFrom::Start(file_position))?;

        let bytes_read = self.decoder.reader.read(&mut buf[..to_read])?;
        self.current_position += bytes_read as u64;

        Ok(bytes_read)
    }
}

impl<R: Read + Seek> Seek for ZipPayloadReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> {
        let entry = match &self.current_entry {
            Some(entry) => entry,
            None => {
                self.load_payload_entry()?;
                self.current_entry.as_ref().unwrap()
            }
        };

        let new_position = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::End(offset) => {
                if offset >= 0 {
                    entry.uncompressed_size + offset as u64
                } else {
                    entry.uncompressed_size.saturating_sub((-offset) as u64)
                }
            }
            SeekFrom::Current(offset) => {
                if offset >= 0 {
                    self.current_position + offset as u64
                } else {
                    self.current_position.saturating_sub((-offset) as u64)
                }
            }
        };

        if new_position > entry.uncompressed_size {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Seek beyond end of data",
            ));
        }

        self.current_position = new_position;
        Ok(self.current_position)
    }
}

impl<R: Read + Seek> ZipDecoder<R> {
    pub fn new(mut reader: R) -> IoResult<Self> {
        let entries = Self::read_central_directory(&mut reader)?;
        Ok(ZipDecoder { reader, entries })
    }

    pub fn get_entry(&self, name: &str) -> Option<&ZipEntry> {
        self.entries.get(name)
    }

    pub fn get_data_offset(&mut self, entry: &ZipEntry) -> IoResult<u64> {
        ZipParser::get_data_offset(&mut self.reader, entry)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))
    }

    fn read_central_directory(reader: &mut R) -> IoResult<HashMap<String, ZipEntry>> {
        let (cd_offset, num_entries) = ZipParser::get_central_directory_info(reader)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;

        reader.seek(SeekFrom::Start(cd_offset))?;
        let mut entries = HashMap::new();

        for _ in 0..num_entries {
            let entry = ZipParser::read_central_directory_entry(reader)
                .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;
            entries.insert(entry.name.clone(), entry);
        }

        Ok(entries)
    }
}
