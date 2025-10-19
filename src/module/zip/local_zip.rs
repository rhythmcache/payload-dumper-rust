use crate::module::zip::zip_core::{ZipEntry, ZipParser};
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::path::Path;

pub struct ZipDecoder<R: Read + Seek> {
    reader: R,
    entries: HashMap<String, ZipEntry>,
}

pub struct ZipPayloadReader<R: Read + Seek> {
    decoder: ZipDecoder<R>,
    current_entry: Option<ZipEntry>,
    current_position: u64,
}

pub type FileZipPayloadReader = ZipPayloadReader<std::fs::File>;

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

    pub fn new_for_parallel<P: AsRef<Path>>(path: P) -> IoResult<Self> {
        Self::from_file(path)
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
