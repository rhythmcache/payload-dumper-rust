use crate::module::http::HttpReader;
use crate::module::zip::zip_core::ZipParser;
use anyhow::Result;
use std::io::{self, Read, Seek, SeekFrom};

pub struct RemoteZipReader {
    pub http_reader: HttpReader,
    payload_offset: u64,
    payload_size: u64,
    current_position: u64,
}

impl RemoteZipReader {
    fn find_payload_via_zip_structure(mut http_reader: HttpReader) -> Result<Self> {
        // Use the traditional ZIP parsing methods for remote readers
        // (memory mapping is not available for HTTP streams)
        let entry = ZipParser::find_payload_entry(&mut http_reader)?;

        // Calculate data offset
        let payload_offset = ZipParser::get_data_offset(&mut http_reader, &entry)?;

        // Verify payload magic
        ZipParser::verify_payload_magic(&mut http_reader, payload_offset)?;

        // Reset position
        http_reader.seek(SeekFrom::Start(payload_offset))?;

        Ok(Self {
            http_reader,
            payload_offset,
            payload_size: entry.compressed_size, // Since we only support stored method
            current_position: 0,
        })
    }

    pub fn new_for_parallel(url: String) -> Result<Self> {
        Self::new_for_parallel_with_user_agent(url, None)
    }

    // Accepts custom user agent
    pub fn new_for_parallel_with_user_agent(url: String, user_agent: Option<&str>) -> Result<Self> {
        let http_reader = HttpReader::new_with_user_agent(url.clone(), user_agent, false)?;
        Self::find_payload_via_zip_structure(http_reader)
    }
}

impl Read for RemoteZipReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.current_position >= self.payload_size {
            return Ok(0);
        }

        let remaining = self.payload_size - self.current_position;
        let to_read = std::cmp::min(buf.len() as u64, remaining) as usize;

        if to_read == 0 {
            return Ok(0);
        }

        let bytes_read = self.http_reader.read_at(
            self.payload_offset + self.current_position,
            &mut buf[..to_read],
        )?;

        self.current_position += bytes_read as u64;

        Ok(bytes_read)
    }
}

impl Seek for RemoteZipReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                if offset >= 0 {
                    self.current_position.saturating_add(offset as u64)
                } else {
                    self.current_position.saturating_sub(offset.abs() as u64)
                }
            }
            SeekFrom::End(offset) => {
                if offset >= 0 {
                    self.payload_size.saturating_add(offset as u64)
                } else {
                    self.payload_size.saturating_sub(offset.abs() as u64)
                }
            }
        };

        if new_pos > self.payload_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Invalid position to seek: {} in size {}",
                    new_pos, self.payload_size
                ),
            ));
        }

        self.current_position = new_pos;
        Ok(self.current_position)
    }
}

// Note: RemoteZipReader does not implement AdvancedReader because:
// 1. HTTP streams cannot be memory-mapped
// 2. get_slice() is not meaningful for network streams
// 3. It already has its own optimized read_at() implementation via HttpReader
//
// This keeps remote functionality working exactly as before while allowing
// local ZIP files to benefit from memory mapping optimizations.
