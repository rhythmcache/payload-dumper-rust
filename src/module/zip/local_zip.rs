use crate::module::zip::advanced_reader::{AdvancedReader, create_optimal_reader};
use crate::module::zip::zip_core::{ZipEntry, ZipParser};
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Read, Result as IoResult, Seek, SeekFrom};
use std::path::Path;

pub struct OptimizedZipDecoder {
    reader: Box<dyn AdvancedReader>,
    entries: HashMap<String, ZipEntry>,
}

pub struct OptimizedZipPayloadReader {
    decoder: OptimizedZipDecoder,
    current_entry: Option<ZipEntry>,
    current_position: u64,
}

pub type FileZipPayloadReader = OptimizedZipPayloadReader;

impl OptimizedZipPayloadReader {
    pub fn new(reader: Box<dyn AdvancedReader>) -> IoResult<Self> {
        let decoder = OptimizedZipDecoder::new(reader)?;
        Ok(OptimizedZipPayloadReader {
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
        let reader = create_optimal_reader(path.as_ref())?;
        let mut zip_reader = Self::new(reader)?;
        zip_reader.load_payload_entry()?;
        Ok(zip_reader)
    }
}

impl Read for OptimizedZipPayloadReader {
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

        // Use optimized read_at method which is much faster for memory-mapped readers
        let bytes_read = self
            .decoder
            .reader
            .read_at(file_position, &mut buf[..to_read])
            .map_err(Error::other)?;

        self.current_position += bytes_read as u64;

        Ok(bytes_read)
    }
}

impl Seek for OptimizedZipPayloadReader {
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

impl OptimizedZipDecoder {
    pub fn new(mut reader: Box<dyn AdvancedReader>) -> IoResult<Self> {
        let entries = Self::read_central_directory(&mut reader)?;
        Ok(OptimizedZipDecoder { reader, entries })
    }

    pub fn get_entry(&self, name: &str) -> Option<&ZipEntry> {
        self.entries.get(name)
    }

    // Get the actual data offset for an entry (after local header)
    pub fn get_data_offset(&mut self, entry: &ZipEntry) -> IoResult<u64> {
        ZipParser::get_data_offset_optimized(&mut *self.reader, entry)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))
    }

    fn read_central_directory(
        reader: &mut Box<dyn AdvancedReader>,
    ) -> IoResult<HashMap<String, ZipEntry>> {
        // Try optimized parsing first (for memory-mapped readers)
        match ZipParser::get_central_directory_info_optimized(&mut **reader) {
            Ok((cd_offset, num_entries)) => {
                let mut entries = HashMap::new();

                // Try memory-mapped parsing for maximum speed
                if reader.supports_zero_copy() {
                    // Estimate central directory size (conservative)
                    let estimated_size = num_entries * 128;

                    if let Some(cd_data) = reader.get_slice(cd_offset, estimated_size) {
                        let mut offset = 0;
                        let mut parsed_entries = 0;

                        while parsed_entries < num_entries && offset < cd_data.len() {
                            match ZipParser::parse_entry_from_slice(cd_data, &mut offset) {
                                Ok(entry) => {
                                    entries.insert(entry.name.clone(), entry);
                                    parsed_entries += 1;
                                }
                                Err(_) => {
                                    // If memory parsing fails, fall back to traditional method
                                    return Self::read_central_directory_traditional(
                                        reader,
                                        cd_offset,
                                        num_entries,
                                    );
                                }
                            }
                        }

                        if parsed_entries == num_entries {
                            //  eprintln!(
                            //      "[DEBUG] Successfully parsed {} entries using memory-mapped zero-copy method",
                            //      num_entries
                            //  );
                            return Ok(entries);
                        }
                    }
                }

                // Fall back to traditional method
                Self::read_central_directory_traditional(reader, cd_offset, num_entries)
            }
            Err(e) => Err(Error::new(ErrorKind::InvalidData, e.to_string())),
        }
    }

    fn read_central_directory_traditional(
        reader: &mut Box<dyn AdvancedReader>,
        cd_offset: u64,
        num_entries: usize,
    ) -> IoResult<HashMap<String, ZipEntry>> {
        reader.seek(SeekFrom::Start(cd_offset))?;
        let mut entries = HashMap::new();

        for _ in 0..num_entries {
            let entry = ZipParser::read_central_directory_entry(&mut **reader)
                .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;
            entries.insert(entry.name.clone(), entry);
        }

        // eprintln!(
        //  "[DEBUG] Parsed {} entries using traditional I/O method",
        // num_entries
        //  );
        Ok(entries)
    }
}
