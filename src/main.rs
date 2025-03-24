use anyhow::{anyhow, Context, Result};
use attohttpc;
use byteorder::{BigEndian, ReadBytesExt};
use bzip2::read::BzDecoder;
use clap::{Parser, ValueEnum};
use digest::Digest;
use hex;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use lzma::LzmaReader;
use memmap2::Mmap;
use num_cpus;
use prost::Message;
use rayon::prelude::*;
use serde::Serialize;
use serde_json;
use sha2::Sha256;
use std::collections::HashSet;
use std::ffi::CStr;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url;

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}
include!(concat!(env!("OUT_DIR"), "/chromeos_update_engine.rs"));
const BSDF2_MAGIC: &[u8] = b"BSDF2";

const BUFFER_SIZE: usize = 5 * 1024 * 1024; // can change

#[link(name = "zip")]
unsafe extern "C" {
    pub fn zip_open(path: *const c_char, flags: c_int, errorp: *mut c_int) -> *mut c_void;
    pub fn zip_close(archive: *mut c_void) -> c_int;
    pub fn zip_get_num_entries(archive: *mut c_void, flags: c_int) -> i64;
    pub fn zip_get_name(archive: *mut c_void, index: i64, flags: c_int) -> *const c_char;
    pub fn zip_stat_index(
        archive: *mut c_void,
        index: u64,
        flags: c_int,
        st: *mut zip_stat_t,
    ) -> c_int;
    pub fn zip_fopen_index(archive: *mut c_void, index: u64, flags: c_int) -> *mut c_void;
    pub fn zip_fclose(file: *mut c_void) -> c_int;
    pub fn zip_fread(file: *mut c_void, buf: *mut c_void, nbytes: usize) -> isize;
    pub fn zip_stat(
        archive: *mut c_void,
        name: *const c_char,
        flags: c_int,
        st: *mut zip_stat_t,
    ) -> c_int;
    pub fn zip_fopen(archive: *mut c_void, name: *const c_char, flags: c_int) -> *mut c_void;
}
#[repr(C)]
pub struct zip_stat_t {
    pub valid: c_uint,
    pub name: *const c_char,
    pub index: c_uint,
    pub size: u64,
}
impl Default for zip_stat_t {
    fn default() -> Self {
        zip_stat_t {
            valid: 0,
            name: std::ptr::null(),
            index: 0,
            size: 0,
        }
    }
}

#[derive(Serialize)]
struct PartitionMetadata {
    partition_name: String,
    size_in_blocks: u64,
    size_in_bytes: u64,
    size_readable: String,
    hash: Option<String>,
    start_offset: u64,
    end_offset: u64,
    data_offset: u64,
    partition_type: String,
    operations_count: usize,
    compression_type: String,
    encryption: String,
    block_size: u64,
    total_blocks: u64,
    run_postinstall: Option<bool>,
    postinstall_path: Option<String>,
    filesystem_type: Option<String>,
    postinstall_optional: Option<bool>,
    hash_tree_algorithm: Option<String>,
    version: Option<String>,
}

#[derive(Serialize)]
struct DynamicPartitionGroupInfo {
    name: String,
    size: Option<u64>,
    partition_names: Vec<String>,
}

#[derive(Serialize)]
struct VabcFeatureSetInfo {
    threaded: Option<bool>,
    batch_writes: Option<bool>,
}

#[derive(Serialize)]
struct DynamicPartitionInfo {
    groups: Vec<DynamicPartitionGroupInfo>,
    snapshot_enabled: Option<bool>,
    vabc_enabled: Option<bool>,
    vabc_compression_param: Option<String>,
    cow_version: Option<u32>,
    vabc_feature_set: Option<VabcFeatureSetInfo>,
    compression_factor: Option<u64>,
}

#[derive(Serialize)]
struct ApexInfoMetadata {
    package_name: Option<String>,
    version: Option<i64>,
    is_compressed: Option<bool>,
    decompressed_size: Option<i64>,
}

#[derive(Serialize)]
struct PayloadMetadata {
    security_patch_level: Option<String>,
    block_size: u32,
    minor_version: u32,
    max_timestamp: Option<i64>,
    dynamic_partition_metadata: Option<DynamicPartitionInfo>,
    partial_update: Option<bool>,
    apex_info: Vec<ApexInfoMetadata>,
    partitions: Vec<PartitionMetadata>,
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(next_line_help = true)]

struct Args {
    payload_path: PathBuf,

    #[arg(
        long,
        default_value = "output",
        help = "Output directory for extracted partitions"
    )]
    out: PathBuf,

    #[arg(long, help = "Enable differential OTA mode (requires --old)")]
    diff: bool,

    #[arg(
        long,
        default_value = "old",
        help = "Path to the directory containing old partition images (required for --diff)"
    )]
    old: PathBuf,

    #[arg(
        long,
        default_value = "",
        hide_default_value = true,
        help = "Comma-separated list of partition names to extract"
    )]
    images: String,

    #[arg(long, help = "Number of threads to use for parallel processing")]
    threads: Option<usize>,

    #[arg(
        value_enum,
        long,
        default_value_t = DecompressMode::All,
        help = "Decompression mode: 'all' for full extraction, 'raw-only' for raw payload.bin extraction"
    )]
    decompress_mode: DecompressMode,

    #[arg(
        long,
        conflicts_with_all = &["out", "diff", "old", "images", "threads", "decompress_mode", "metadata"],
        help = "List available partitions in the payload and save metadata as JSON"
    )]
    list: bool,

    #[arg(
        long,
        help = "Save Complete Metadata as JSON",
        conflicts_with_all = &["list", "diff", "old", "images", "decompress_mode"]
    )]
    metadata: bool,


    #[arg(
        long,
        hide = true,
        help = "Disable parallel processing (useful for debugging)"
    )]
    no_parallel: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum DecompressMode {
    All,
    RawOnly,
}
impl fmt::Display for DecompressMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecompressMode::All => write!(f, "all"),
            DecompressMode::RawOnly => write!(f, "raw-only"),
        }
    }
}

struct LibZipReader {
    archive: *mut c_void,
    file: *mut c_void,
    current_position: u64,
    file_size: u64,
    mmap: Option<Mmap>,
    buffer: Vec<u8>,
    buffer_size: usize,
    path: String,
}

impl LibZipReader {
    fn new(archive: *mut c_void, path: String) -> Result<Self> {
        let buffer_size = 6 * 1024 * 1024; // can change this

        unsafe {
            let payload_name = CStr::from_bytes_with_nul(b"payload.bin\0")
                .map_err(|_| anyhow!("Failed to create CStr for payload.bin"))?;

            let mut stat = zip_stat_t::default();
            if zip_stat(archive, payload_name.as_ptr(), 0, &mut stat) == 0 {
                let file = zip_fopen(archive, payload_name.as_ptr(), 0);
                if !file.is_null() {
                    let file_size = stat.size;
                    let mut buffer = Vec::with_capacity(buffer_size);
                    buffer.resize(buffer_size, 0);

                    return Ok(Self {
                        archive,
                        file,
                        current_position: 0,
                        file_size,
                        mmap: None,
                        buffer,
                        buffer_size,
                        path,
                    });
                }
            }

            // fallback to dynamic search if payload.bin is not at the root
            // it is unnecessary though
            let num_entries = zip_get_num_entries(archive, 0);
            let mut file_index = -1;
            for i in 0..num_entries {
                let name = zip_get_name(archive, i, 0);
                if name.is_null() {
                    continue;
                }
                let name_str = CStr::from_ptr(name).to_string_lossy();
                if name_str.ends_with("payload.bin") {
                    file_index = i;
                    break;
                }
            }
            if file_index == -1 {
                zip_close(archive);
                return Err(anyhow!("payload.bin not found in ZIP file"));
            }
            if zip_stat_index(archive, file_index as u64, 0, &mut stat) != 0 {
                zip_close(archive);
                return Err(anyhow!("Failed to get file stats"));
            }
            let file = zip_fopen_index(archive, file_index as u64, 0);
            if file.is_null() {
                zip_close(archive);
                return Err(anyhow!("Failed to open payload.bin in ZIP"));
            }
            let file_size = stat.size;
            let mut buffer = Vec::with_capacity(buffer_size);
            buffer.resize(buffer_size, 0);

            Ok(Self {
                archive,
                file,
                current_position: 0,
                file_size,
                mmap: None,
                buffer,
                buffer_size,
                path,
            })
        }
    }
    fn new_for_parallel(path: String) -> Result<Self> {
        unsafe {
            let mut error = 0;
            let archive = zip_open(path.as_ptr() as *const c_char, 0, &mut error);
            if archive.is_null() {
                return Err(anyhow!("Failed to open ZIP file: error {}", error));
            }
            Self::new(archive, path)
        }
    }
}

impl Drop for LibZipReader {
    fn drop(&mut self) {
        unsafe {
            if !self.file.is_null() {
                zip_fclose(self.file);
            }
            if !self.archive.is_null() {
                zip_close(self.archive);
            }
        }
    }
}
impl Read for LibZipReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(mmap) = &self.mmap {
            let start = self.current_position as usize;
            let end = (self.current_position + buf.len() as u64) as usize;

            if start >= mmap.len() {
                return Ok(0);
            }

            let end = end.min(mmap.len());
            let bytes_to_read = end - start;

            buf[..bytes_to_read].copy_from_slice(&mmap[start..end]);
            self.current_position += bytes_to_read as u64;

            return Ok(bytes_to_read);
        }
        unsafe {
            let read_bytes = zip_fread(self.file, buf.as_mut_ptr() as *mut c_void, buf.len());
            if read_bytes < 0 {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Failed to read from ZIP",
                ));
            }

            self.current_position += read_bytes as u64;
            Ok(read_bytes as usize)
        }
    }
}
impl Seek for LibZipReader {
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
                    self.file_size.saturating_add(offset as u64)
                } else {
                    self.file_size.saturating_sub(offset.abs() as u64)
                }
            }
        };
        if new_pos > self.file_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Attempted to seek past end of file",
            ));
        }
        if self.mmap.is_some() {
            self.current_position = new_pos;
            return Ok(self.current_position);
        }
        if new_pos == self.current_position {
            return Ok(new_pos);
        }
        unsafe {
            zip_fclose(self.file);
            let num_entries = zip_get_num_entries(self.archive, 0);
            let mut file_index = -1;

            for i in 0..num_entries {
                let name = zip_get_name(self.archive, i, 0);
                if name.is_null() {
                    continue;
                }

                let name_str = CStr::from_ptr(name).to_string_lossy();
                if name_str.ends_with("payload.bin") {
                    file_index = i;
                    break;
                }
            }
            self.file = zip_fopen_index(self.archive, file_index as u64, 0);
            if self.file.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Failed to reopen file for seeking",
                ));
            }
            let mut skip_buf = vec![0u8; self.buffer_size];
            let mut remaining = new_pos;
            self.current_position = 0;

            while remaining > 0 {
                let to_read = remaining.min(skip_buf.len() as u64) as usize;
                let read_bytes =
                    zip_fread(self.file, skip_buf.as_mut_ptr() as *mut c_void, to_read);
                if read_bytes <= 0 {
                    return Err(io::Error::new(io::ErrorKind::Other, "Failed to seek"));
                }
                self.current_position += read_bytes as u64;
                remaining -= read_bytes as u64;

                if read_bytes < to_read as isize {
                    break;
                }
            }
            Ok(self.current_position)
        }
    }
}
struct HttpReader {
    url: String,
    position: u64,
    content_length: u64,
    client: attohttpc::Session,
    buffer: Vec<u8>,
    buffer_start: u64,
    buffer_end: u64,
    buffer_size: usize,
    content_type: Option<String>,
}

impl Clone for HttpReader {
    fn clone(&self) -> Self {
        let mut client = attohttpc::Session::new();
        client.timeout(Duration::from_secs(300));
        client.header("Accept-Encoding", "gzip, deflate");
        client.header("Accept-Ranges", "bytes");
        client.header("Connection", "keep-alive");
        client.header("Cache-Control", "no-transform");

        Self {
            url: self.url.clone(),
            position: self.position,
            content_length: self.content_length,
            client,
            buffer: self.buffer.clone(),
            buffer_start: self.buffer_start,
            buffer_end: self.buffer_end,
            buffer_size: self.buffer_size,
            content_type: self.content_type.clone(),
        }
    }
}

impl HttpReader {
    fn new(url: String) -> Result<Self> {
        Self::new_internal(url, true)
    }

    fn new_silent(url: String) -> Result<Self> {
        Self::new_internal(url, false)
    }

    fn new_internal(url: String, print_size: bool) -> Result<Self> {
        let mut client = attohttpc::Session::new();
        client.timeout(Duration::from_secs(600));
        client.header("Accept-Encoding", "*");
        client.header("Accept", "*/*");
        client.header("User-Agent", "Mozilla/5.0");
        client.header("Accept-Ranges", "bytes");
        client.header("Connection", "keep-alive");
        client.header("Cache-Control", "no-transform");

        let parsed_url = url::Url::parse(&url).map_err(|e| anyhow!("Invalid URL: {}", e))?;

        let _host = parsed_url
            .host_str()
            .ok_or_else(|| anyhow!("No host in URL"))?;
        let _port = parsed_url
            .port()
            .unwrap_or(if parsed_url.scheme() == "https" {
                443
            } else {
                80
            });

        let mut retry_count = 0;
        let max_retries = 3;
        let mut last_error = None;

        while retry_count < max_retries {
            match client.head(&url).send() {
                Ok(response) => {
                    let content_type = response
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());

                    let content_length = response
                        .headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .ok_or_else(|| anyhow!("Could not determine content length"))?;

                    if print_size {
                        println!("- File size: {}", format_size(content_length));
                    }

                    return Ok(Self {
                        url,
                        position: 0,
                        content_length,
                        client,
                        buffer: Vec::with_capacity(BUFFER_SIZE),
                        buffer_start: 0,
                        buffer_end: 0,
                        buffer_size: BUFFER_SIZE,
                        content_type,
                    });
                }
                Err(e) => {
                    last_error = Some(e);
                    retry_count += 1;
                    if retry_count < max_retries {
                        std::thread::sleep(Duration::from_secs(2 * retry_count as u64));
                    }
                }
            }
        }

        Err(anyhow!(
            "Failed to connect after {} retries. Last error: {}",
            max_retries,
            last_error.unwrap()
        ))
    }

    fn fill_buffer(&mut self) -> io::Result<()> {
        let start = self.position;
        let end = std::cmp::min(start + self.buffer_size as u64 - 1, self.content_length - 1);

        if start >= self.content_length {
            return Ok(());
        }

        let range = format!("bytes={}-{}", start, end);

        let mut retry_count = 0;
        let max_retries = 3;

        while retry_count < max_retries {
            match self
                .client
                .get(&self.url)
                .header("Range", range.clone())
                .header("Connection", "keep-alive")
                .header("Cache-Control", "no-transform")
                .send()
            {
                Ok(mut response) => {
                    if !response.status().is_success() {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!("Failed to access URL range: {}", response.status()),
                        ));
                    }

                    self.buffer.clear();
                    let mut temp_buf = vec![0u8; self.buffer_size];
                    let mut total_read = 0;

                    loop {
                        match response.read(&mut temp_buf[total_read..]) {
                            Ok(0) => break,
                            Ok(n) => {
                                total_read += n;
                                if total_read == temp_buf.len() {
                                    break;
                                }
                            }
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                            Err(e) => return Err(e),
                        }
                    }

                    self.buffer.extend_from_slice(&temp_buf[..total_read]);
                    self.buffer_start = start;
                    self.buffer_end = start + total_read as u64;
                    return Ok(());
                }
                Err(e) => {
                    retry_count += 1;
                    if retry_count == max_retries {
                        return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
                    }
                    std::thread::sleep(Duration::from_secs(2 * retry_count as u64));
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to fill buffer after maximum retries",
        ))
    }
}

impl Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.content_length {
            return Ok(0);
        }
        if self.position < self.buffer_start || self.position >= self.buffer_end {
            self.fill_buffer()?;
        }

        let buffer_offset = (self.position - self.buffer_start) as usize;
        let available = (self.buffer_end - self.position) as usize;
        let to_read = std::cmp::min(buf.len(), available);

        buf[..to_read].copy_from_slice(&self.buffer[buffer_offset..buffer_offset + to_read]);
        self.position += to_read as u64;

        Ok(to_read)
    }
}
impl Seek for HttpReader {
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
                if offset >= 0 {
                    self.content_length.saturating_add(offset as u64)
                } else {
                    self.content_length.saturating_sub(offset.abs() as u64)
                }
            }
        };
        if new_pos > self.content_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Attempted to seek past end of file",
            ));
        }
        self.position = new_pos;
        Ok(self.position)
    }
}
struct RemoteZipReader {
    http_reader: HttpReader,
    payload_offset: u64,
    payload_size: u64,
    current_position: u64,
}

impl RemoteZipReader {
    fn new(url: String) -> Result<Self> {
        let http_reader = HttpReader::new(url.clone())?;
        if let Ok(payload_info) = Self::find_payload_via_metadata(&mut http_reader.clone()) {
            return Ok(Self {
                http_reader,
                payload_offset: payload_info.0,
                payload_size: payload_info.1,
                current_position: 0,
            });
        }
        Self::find_payload_via_zip_structure(http_reader)
    }
    fn find_payload_via_metadata(http_reader: &mut HttpReader) -> Result<(u64, u64)> {
        let search_size = std::cmp::min(http_reader.content_length, 131072);
        http_reader.seek(SeekFrom::End(-(search_size as i64)))?;

        let mut tail_buffer = vec![0u8; search_size as usize];
        let bytes_read = http_reader.read(&mut tail_buffer)?;
        tail_buffer.truncate(bytes_read);

        let search_pattern = b"payload.bin:";
        for i in 0..tail_buffer.len().saturating_sub(search_pattern.len()) {
            if tail_buffer[i..].starts_with(search_pattern) {
                let line_end = i + tail_buffer[i..]
                    .iter()
                    .position(|&b| b == b',')
                    .unwrap_or_else(|| tail_buffer[i..].len());

                let property_line = &tail_buffer[i..line_end];
                let values: Vec<&[u8]> = property_line.split(|&b| b == b':').collect();

                if values.len() >= 3 {
                    if let (Ok(offset_str), Ok(size_str)) = (
                        std::str::from_utf8(values[1]),
                        std::str::from_utf8(values[2]),
                    ) {
                        if let (Ok(offset), Ok(size)) =
                            (offset_str.parse::<u64>(), size_str.parse::<u64>())
                        {
                            http_reader.seek(SeekFrom::Start(offset))?;
                            let mut header = [0u8; 4];
                            http_reader.read_exact(&mut header)?;

                            if header[0] == 0x50 && header[1] == 0x4B {
                                return Ok((offset, size));
                            }
                        }
                    }
                }
            }
        }
        Err(anyhow!("Could not find payload.bin metadata"))
    }
    fn find_payload_via_zip_structure(mut http_reader: HttpReader) -> Result<Self> {
        let search_size = std::cmp::min(http_reader.content_length, 1024 * 1024); // increased to 1MB
        http_reader.seek(SeekFrom::End(-(search_size as i64)))?;

        let mut ecd_buffer = vec![0u8; search_size as usize];
        let bytes_read = http_reader.read(&mut ecd_buffer)?;
        ecd_buffer.truncate(bytes_read);
        let mut eocd_offset = None;
        for i in (0..ecd_buffer.len().saturating_sub(22)).rev() {
            if ecd_buffer[i] == 0x50
                && ecd_buffer[i + 1] == 0x4B
                && ecd_buffer[i + 2] == 0x05
                && ecd_buffer[i + 3] == 0x06
            {
                eocd_offset = Some(i);
                break;
            }
        }

        let eocd_offset =
            eocd_offset.ok_or_else(|| anyhow!("Could not find End of Central Directory record"))?;
        let cd_offset = u32::from_le_bytes([
            ecd_buffer[eocd_offset + 16],
            ecd_buffer[eocd_offset + 17],
            ecd_buffer[eocd_offset + 18],
            ecd_buffer[eocd_offset + 19],
        ]);

        let num_entries =
            u16::from_le_bytes([ecd_buffer[eocd_offset + 10], ecd_buffer[eocd_offset + 11]]);
        let (real_cd_offset, real_num_entries) = if cd_offset == 0xFFFFFFFF {
            let mut zip64_locator_offset = None;
            let search_start = if eocd_offset >= 20 {
                eocd_offset - 20
            } else {
                0
            };
            let search_end = eocd_offset;

            for i in (search_start..search_end).rev() {
                if ecd_buffer[i] == 0x50
                    && ecd_buffer[i + 1] == 0x4B
                    && ecd_buffer[i + 2] == 0x06
                    && ecd_buffer[i + 3] == 0x07
                {
                    zip64_locator_offset = Some(i);
                    break;
                }
            }

            let zip64_locator_offset =
                zip64_locator_offset.ok_or_else(|| anyhow!("Could not find ZIP64 EOCD locator"))?;
            let zip64_eocd_offset = u64::from_le_bytes([
                ecd_buffer[zip64_locator_offset + 8],
                ecd_buffer[zip64_locator_offset + 9],
                ecd_buffer[zip64_locator_offset + 10],
                ecd_buffer[zip64_locator_offset + 11],
                ecd_buffer[zip64_locator_offset + 12],
                ecd_buffer[zip64_locator_offset + 13],
                ecd_buffer[zip64_locator_offset + 14],
                ecd_buffer[zip64_locator_offset + 15],
            ]);
            http_reader.seek(SeekFrom::Start(zip64_eocd_offset))?;
            let mut zip64_eocd = [0u8; 56];
            http_reader.read_exact(&mut zip64_eocd)?;

            if zip64_eocd[0] != 0x50
                || zip64_eocd[1] != 0x4B
                || zip64_eocd[2] != 0x06
                || zip64_eocd[3] != 0x06
            {
                return Err(anyhow!("Invalid ZIP64 EOCD signature"));
            }

            let cd_offset = u64::from_le_bytes([
                zip64_eocd[48],
                zip64_eocd[49],
                zip64_eocd[50],
                zip64_eocd[51],
                zip64_eocd[52],
                zip64_eocd[53],
                zip64_eocd[54],
                zip64_eocd[55],
            ]);

            let num_entries = u64::from_le_bytes([
                zip64_eocd[32],
                zip64_eocd[33],
                zip64_eocd[34],
                zip64_eocd[35],
                zip64_eocd[36],
                zip64_eocd[37],
                zip64_eocd[38],
                zip64_eocd[39],
            ]);

            (cd_offset, num_entries as usize)
        } else {
            (cd_offset as u64, num_entries as usize)
        };

        http_reader.seek(SeekFrom::Start(real_cd_offset))?;

        for _entry_num in 0..real_num_entries {
            //1
            let mut entry_header = [0u8; 46];
            http_reader.read_exact(&mut entry_header)?;
            if entry_header[0] != 0x50
                || entry_header[1] != 0x4B
                || entry_header[2] != 0x01
                || entry_header[3] != 0x02
            {
                return Err(anyhow!("Invalid central directory header signature"));
            }

            let filename_len = u16::from_le_bytes([entry_header[28], entry_header[29]]) as usize;
            let extra_len = u16::from_le_bytes([entry_header[30], entry_header[31]]) as usize;
            let comment_len = u16::from_le_bytes([entry_header[32], entry_header[33]]) as usize;

            let mut local_header_offset = u32::from_le_bytes([
                entry_header[42],
                entry_header[43],
                entry_header[44],
                entry_header[45],
            ]) as u64;

            let mut compressed_size = u32::from_le_bytes([
                entry_header[20],
                entry_header[21],
                entry_header[22],
                entry_header[23],
            ]) as u64;
            let mut filename = vec![0u8; filename_len];
            http_reader.read_exact(&mut filename)?;
            let mut extra_data = vec![0u8; extra_len];
            http_reader.read_exact(&mut extra_data)?;
            http_reader.seek(SeekFrom::Current(comment_len as i64))?;
            if local_header_offset == 0xFFFFFFFF || compressed_size == 0xFFFFFFFF {
                let mut pos = 0;
                while pos + 4 <= extra_data.len() {
                    let header_id = u16::from_le_bytes([extra_data[pos], extra_data[pos + 1]]);
                    let data_size =
                        u16::from_le_bytes([extra_data[pos + 2], extra_data[pos + 3]]) as usize;

                    if header_id == 0x0001 && pos + 4 + data_size <= extra_data.len() {
                        let mut field_pos = pos + 4;

                        if local_header_offset == 0xFFFFFFFF && field_pos + 8 <= pos + 4 + data_size
                        {
                            local_header_offset = u64::from_le_bytes([
                                extra_data[field_pos],
                                extra_data[field_pos + 1],
                                extra_data[field_pos + 2],
                                extra_data[field_pos + 3],
                                extra_data[field_pos + 4],
                                extra_data[field_pos + 5],
                                extra_data[field_pos + 6],
                                extra_data[field_pos + 7],
                            ]);
                            field_pos += 8;
                        }

                        if compressed_size == 0xFFFFFFFF && field_pos + 8 <= pos + 4 + data_size {
                            compressed_size = u64::from_le_bytes([
                                extra_data[field_pos],
                                extra_data[field_pos + 1],
                                extra_data[field_pos + 2],
                                extra_data[field_pos + 3],
                                extra_data[field_pos + 4],
                                extra_data[field_pos + 5],
                                extra_data[field_pos + 6],
                                extra_data[field_pos + 7],
                            ]);
                        }
                    }
                    pos += 4 + data_size;
                }
            }

            if filename == b"payload.bin" || filename.ends_with(b"/payload.bin") {
                http_reader.seek(SeekFrom::Start(local_header_offset))?;
                let mut local_header = [0u8; 30];
                http_reader.read_exact(&mut local_header)?;

                if local_header[0] != 0x50
                    || local_header[1] != 0x4B
                    || local_header[2] != 0x03
                    || local_header[3] != 0x04
                {
                    return Err(anyhow!("Invalid local file header signature"));
                }

                let local_filename_len =
                    u16::from_le_bytes([local_header[26], local_header[27]]) as u64;
                let local_extra_len =
                    u16::from_le_bytes([local_header[28], local_header[29]]) as u64;

                let payload_offset =
                    local_header_offset + 30 + local_filename_len + local_extra_len;

                http_reader.seek(SeekFrom::Start(payload_offset))?;
                let mut magic = [0u8; 4];
                http_reader.read_exact(&mut magic)?;

                if magic != *b"CrAU" {
                    return Err(anyhow!(
                        "Invalid payload file: magic 'CrAU' not found at calculated offset"
                    ));
                }

                http_reader.seek(SeekFrom::Start(payload_offset))?;

                return Ok(Self {
                    http_reader,
                    payload_offset,
                    payload_size: compressed_size,
                    current_position: 0,
                });
            }
        }

        Err(anyhow!("Could not find payload.bin in ZIP file"))
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

        self.http_reader
            .seek(SeekFrom::Start(self.payload_offset + self.current_position))?;

        let bytes_read = self.http_reader.read(&mut buf[..to_read])?;

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
fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.2} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.2} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.2} KB", size as f64 / KB as f64)
    } else {
        format!("{} bytes", size)
    }
}
fn list_partitions(payload_reader: &mut Box<dyn ReadSeek>) -> Result<()> {
    let mut magic = [0u8; 4];
    payload_reader.read_exact(&mut magic)?;
    if magic != *b"CrAU" {
        payload_reader.seek(SeekFrom::Start(0))?;
        let mut buffer = [0u8; 1024];
        let mut offset = 0;
        while offset < 1024 * 1024 {
            let bytes_read = payload_reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            for i in 0..bytes_read - 3 {
                if buffer[i] == b'C'
                    && buffer[i + 1] == b'r'
                    && buffer[i + 2] == b'A'
                    && buffer[i + 3] == b'U'
                {
                    payload_reader.seek(SeekFrom::Start(offset + i as u64))?;
                    return list_partitions(payload_reader);
                }
            }
            offset += bytes_read as u64;
        }
        return Err(anyhow!("Invalid payload file: magic 'CrAU' not found"));
    }

    let file_format_version = payload_reader.read_u64::<BigEndian>()?;
    if file_format_version != 2 {
        return Err(anyhow!(
            "Unsupported payload version: {}",
            file_format_version
        ));
    }
    let manifest_size = payload_reader.read_u64::<BigEndian>()?;
    let _metadata_signature_size = payload_reader.read_u32::<BigEndian>()?;

    let mut manifest = vec![0u8; manifest_size as usize];
    payload_reader.read_exact(&mut manifest)?;
    let manifest = DeltaArchiveManifest::decode(&manifest[..])?;

    // Display security patch level if available
    if let Some(security_patch) = &manifest.security_patch_level {
        println!("\nSecurity Patch Level: {}\n", security_patch);
    }

    println!("{:<20} {:<15}", "Partition Name", "Size");
    println!("{}", "-".repeat(35));
    for partition in &manifest.partitions {
        let size = partition
            .new_partition_info
            .as_ref()
            .and_then(|info| info.size)
            .unwrap_or(0);
        println!(
            "{:<20} {:<15}",
            partition.partition_name,
            if size > 0 {
                format_size(size)
            } else {
                "Unknown".to_string()
            }
        );
    }
    Ok(())
}
fn bsdf2_read_patch<R: Read>(reader: &mut R) -> Result<(i64, Vec<(i64, i64, i64)>, Vec<u8>)> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;

    let (alg_control, alg_diff, _) = if magic.starts_with(BSDF2_MAGIC) {
        (magic[5], magic[6], magic[7])
    } else {
        return Err(anyhow!("Incorrect BSDF2 magic header"));
    };
    let len_control = reader.read_i64::<BigEndian>()?;
    let len_diff = reader.read_i64::<BigEndian>()?;
    let len_dst = reader.read_i64::<BigEndian>()?;

    let mut control_data = vec![0u8; len_control as usize];
    reader.read_exact(&mut control_data)?;
    let control_data = match alg_control {
        0 => control_data,
        1 => {
            let mut decoder = BzDecoder::new(&control_data[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            decompressed
        }
        2 => {
            let mut decompressed = Vec::new();
            let decompressed_size = brotli::Decompressor::new(&control_data[..], 4096)
                .read_to_end(&mut decompressed)?;
            decompressed.truncate(decompressed_size);
            decompressed
        }
        3 => match zstd::decode_all(Cursor::new(&control_data)) {
            Ok(decompressed) => decompressed,
            Err(e) => {
                return Err(anyhow!(
                    "Failed to decompress control data with Zstd: {}",
                    e
                ));
            }
        },
        _ => {
            return Err(anyhow!(
                "Unsupported control compression algorithm: {}",
                alg_control
            ));
        }
    };
    let mut control = Vec::new();
    let mut i = 0;
    while i < control_data.len() {
        if i + 24 > control_data.len() {
            break;
        }
        let x = i64::from_be_bytes([
            control_data[i],
            control_data[i + 1],
            control_data[i + 2],
            control_data[i + 3],
            control_data[i + 4],
            control_data[i + 5],
            control_data[i + 6],
            control_data[i + 7],
        ]);
        let y = i64::from_be_bytes([
            control_data[i + 8],
            control_data[i + 9],
            control_data[i + 10],
            control_data[i + 11],
            control_data[i + 12],
            control_data[i + 13],
            control_data[i + 14],
            control_data[i + 15],
        ]);
        let z = i64::from_be_bytes([
            control_data[i + 16],
            control_data[i + 17],
            control_data[i + 18],
            control_data[i + 19],
            control_data[i + 20],
            control_data[i + 21],
            control_data[i + 22],
            control_data[i + 23],
        ]);
        control.push((x, y, z));
        i += 24;
    }
    let mut diff_data = vec![0u8; len_diff as usize];
    reader.read_exact(&mut diff_data)?;
    let diff_data = match alg_diff {
        0 => diff_data,
        1 => {
            let mut decoder = BzDecoder::new(&diff_data[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            decompressed
        }
        2 => {
            let mut decompressed = Vec::new();
            let decompressed_size =
                brotli::Decompressor::new(&diff_data[..], 4096).read_to_end(&mut decompressed)?;
            decompressed.truncate(decompressed_size);
            decompressed
        }
        _ => {
            return Err(anyhow!(
                "Unsupported diff compression algorithm: {}",
                alg_diff
            ));
        }
    };
    Ok((len_dst, control, diff_data))
}
fn bspatch(old_data: &[u8], patch_data: &[u8]) -> Result<Vec<u8>> {
    let mut reader = Cursor::new(patch_data);
    let (len_dst, control, diff_data) = bsdf2_read_patch(&mut reader)?;

    let mut new_data = vec![0u8; len_dst as usize];
    let mut old_pos = 0;
    let mut new_pos = 0;

    for (diff_len, _, seek_adjustment) in control {
        if new_pos + diff_len as usize > new_data.len()
            || old_pos + diff_len as usize > old_data.len()
        {
            return Err(anyhow!("Invalid bspatch control data"));
        }

        for i in 0..diff_len as usize {
            if old_pos + i < old_data.len() && new_pos + i < new_data.len() && i < diff_data.len() {
                new_data[new_pos + i] = old_data[old_pos + i].wrapping_add(diff_data[i]);
            }
        }
        new_pos += diff_len as usize;
        old_pos += diff_len as usize;

        old_pos = (old_pos as i64 + seek_adjustment) as usize;
    }

    Ok(new_data)
}
fn verify_hash(data: &[u8], expected_hash: &[u8]) -> bool {
    if expected_hash.is_empty() {
        return true;
    }
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();

    hash.as_slice() == expected_hash
}
fn process_operation(
    operation_index: usize,
    op: &InstallOperation,
    data_offset: u64,
    block_size: u64,
    payload_file: &mut (impl Read + Seek),
    out_file: &mut (impl Write + Seek),
    old_file: Option<&mut dyn ReadSeek>,
) -> Result<()> {
    payload_file.seek(SeekFrom::Start(data_offset + op.data_offset.unwrap_or(0)))?;
    let mut data = vec![0u8; op.data_length.unwrap_or(0) as usize];
    payload_file.read_exact(&mut data)?;

    if let Some(expected_hash) = op.data_sha256_hash.as_deref() {
        if !verify_hash(&data, expected_hash) {
            println!(
                "⚠️  Warning: Operation {} data hash mismatch.",
                operation_index
            );
            return Ok(());
        }
    }
    match op.r#type() {
        install_operation::Type::ReplaceXz => {
            let mut decompressed = Vec::new();
            match LzmaReader::new_decompressor(Cursor::new(&data)) {
                Ok(mut decompressor) => {
                    if let Err(e) = decompressor.read_to_end(&mut decompressed) {
                        println!(
                            "⚠️  Warning: Failed to decompress XZ in operation {}.  : {}",
                            operation_index, e
                        );
                        return Ok(());
                    }
                    out_file.seek(SeekFrom::Start(
                        op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                    ))?;
                    out_file.write_all(&decompressed)?;
                }
                Err(e) => {
                    println!(
                        "⚠️  Warning: Skipping operation {} due to XZ decompression error.  : {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Zstd => match zstd::decode_all(Cursor::new(&data)) {
            Ok(decompressed) => {
                let mut pos = 0;
                for ext in &op.dst_extents {
                    let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                    let end_pos = pos + ext_size;

                    if end_pos <= decompressed.len() {
                        out_file
                            .seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                        out_file.write_all(&decompressed[pos..end_pos])?;
                        pos = end_pos;
                    } else {
                        println!(
                            "⚠️  Warning: Skipping extent in operation {} due to insufficient decompressed data.",
                            operation_index
                        );
                        break;
                    }
                }
            }
            Err(e) => {
                println!(
                    "⚠️  Warning: Skipping operation {} due to unknown Zstd format: {}",
                    operation_index, e
                );
                return Ok(());
            }
        },
        install_operation::Type::ReplaceBz => {
            let mut decoder = BzDecoder::new(Cursor::new(&data));
            let mut decompressed = Vec::new();
            match decoder.read_to_end(&mut decompressed) {
                Ok(_) => {
                    out_file.seek(SeekFrom::Start(
                        op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                    ))?;
                    out_file.write_all(&decompressed)?;
                }
                Err(e) => {
                    println!(
                        "⚠️  Warning: Skipping operation {} due to unknown BZ2 format.  : {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Replace => {
            out_file.seek(SeekFrom::Start(
                op.dst_extents[0].start_block.unwrap_or(0) * block_size,
            ))?;
            out_file.write_all(&data)?;
        }
        install_operation::Type::SourceCopy => {
            let old_file = old_file
                .ok_or_else(|| anyhow!("SOURCE_COPY supported only for differential OTA"))?;
            out_file.seek(SeekFrom::Start(
                op.dst_extents[0].start_block.unwrap_or(0) * block_size,
            ))?;
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                out_file.write_all(&buffer)?;
            }
        }
        install_operation::Type::SourceBsdiff | install_operation::Type::BrotliBsdiff => {
            let old_file =
                old_file.ok_or_else(|| anyhow!("BSDIFF supported only for differential OTA"))?;

            let mut old_data = Vec::new();
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                old_data.extend_from_slice(&buffer);
            }
            let new_data = match bspatch(&old_data, &data) {
                Ok(new_data) => new_data,
                Err(e) => {
                    println!(
                        "⚠️  Warning: Skipping operation {} due to failed BSDIFF patch.  : {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            };
            let mut pos = 0;
            for ext in &op.dst_extents {
                let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                let end_pos = pos + ext_size;
                if end_pos <= new_data.len() {
                    out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                    out_file.write_all(&new_data[pos..end_pos])?;
                    pos = end_pos;
                } else {
                    println!(
                        "⚠️  Warning: Skipping operation {} due to insufficient patched data.  .",
                        operation_index
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Zero => {
            let zeros = vec![0u8; block_size as usize];
            for ext in &op.dst_extents {
                out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                for _ in 0..ext.num_blocks.unwrap_or(0) {
                    out_file.write_all(&zeros)?;
                }
            }
        }
        _ => {
            println!(
                "⚠️  Warning: Skipping operation {} due to unknown compression method",
                operation_index
            );
            return Ok(());
        }
    }
    Ok(())
}
fn dump_partition(
    partition: &PartitionUpdate,
    data_offset: u64,
    block_size: u64,
    args: &Args,
    payload_file: &mut (impl Read + Seek),
    multi_progress: Option<&MultiProgress>,
) -> Result<()> {
    let partition_name = &partition.partition_name;
    let total_ops = partition.operations.len() as u64;
    let progress_bar = if let Some(mp) = multi_progress {
        let pb = mp.add(ProgressBar::new(100));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/white}] {percent}% - {msg}")
            .unwrap()
            .progress_chars("▰▱"));
        pb.enable_steady_tick(Duration::from_millis(500));
        pb.set_message(format!("Processing {} ({} ops)", partition_name, total_ops));
        Some(pb)
    } else {
        None
    };
    let out_dir = &args.out;
    fs::create_dir_all(out_dir)?;
    let out_path = out_dir.join(format!("{}.img", partition_name));
    let mut out_file = File::create(&out_path)?;

    if let Some(info) = &partition.new_partition_info {
        if info.size.unwrap_or(0) > 0 {
            #[cfg(target_family = "unix")]
            {
                use std::os::unix::fs::FileExt;
                if let Some(size) = info.size {
                    out_file.set_len(size)?;
                } else {
                    return Err(anyhow!("Partition size is missing"));
                }
            }
        }
    }
    let mut old_file = if args.diff {
        let old_path = args.old.join(format!("{}.img", partition_name));
        Some(
            File::open(&old_path)
                .with_context(|| format!("Failed to open original image: {:?}", old_path))?,
        )
    } else {
        None
    };
    for (i, op) in partition.operations.iter().enumerate() {
        process_operation(
            i,
            op,
            data_offset,
            block_size,
            payload_file,
            &mut out_file,
            old_file.as_mut().map(|f| f as &mut dyn ReadSeek),
        )?;

        if let Some(pb) = &progress_bar {
            let percentage = ((i + 1) as f64 / total_ops as f64 * 100.0) as u64;
            pb.set_position(percentage);
        }
    }
    if let Some(pb) = progress_bar {
        pb.finish_with_message(format!(
            "✓ Completed {} ({} ops)",
            partition_name, total_ops
        ));
    }
    drop(out_file);
    let mut out_file = File::open(&out_path)
        .with_context(|| format!("Failed to reopen {} for hash verification", partition_name))?;
    if let Some(info) = &partition.new_partition_info {
        if info.hash.as_ref().map_or(true, |hash| hash.is_empty()) {
            let hash_pb = if let Some(mp) = multi_progress {
                let pb = mp.add(ProgressBar::new_spinner());
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner:.green} {msg}")
                        .unwrap(),
                );
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message(format!("Verifying hash for {}", partition_name));
                Some(pb)
            } else {
                None
            };
            out_file.seek(SeekFrom::Start(0))?;
            let mut hasher = Sha256::new();
            io::copy(&mut out_file, &mut hasher)?;
            let hash = hasher.finalize();
            if let Some(pb) = hash_pb {
                if hash.as_slice() != info.hash.as_deref().unwrap_or(&[]) {
                    pb.finish_with_message(format!("✕ Hash mismatch for {}", partition_name));
                } else {
                    pb.finish_with_message(format!("✓ Hash verified for {}", partition_name));
                }
            }
        }
    }
    Ok(())
}
fn create_payload_reader(path: &PathBuf) -> Result<Box<dyn ReadSeek>> {
    let file = File::open(path)?;
    Ok(Box::new(file) as Box<dyn ReadSeek>)
}
fn save_metadata(
    manifest: &DeltaArchiveManifest,
    output_dir: &PathBuf,
    data_offset: u64,
) -> Result<()> {
    let mut partitions = Vec::new();
    for partition in &manifest.partitions {
        if let Some(info) = &partition.new_partition_info {
            let size_in_bytes = info.size.unwrap_or(0);
            let block_size = manifest.block_size.unwrap_or(4096) as u64;
            let size_in_blocks = size_in_bytes / block_size;
            let total_blocks = size_in_bytes / block_size;
            let hash = info.hash.as_ref().map(|hash| hex::encode(hash));
            let mut start_offset = data_offset;
            for op in &partition.operations {
                if let Some(_first_extent) = op.dst_extents.first() {
                    //2
                    start_offset = data_offset + op.data_offset.unwrap_or(0);
                    break;
                }
            }
            let end_offset = start_offset + size_in_bytes;
            let compression_type = partition
                .operations
                .iter()
                .find_map(|op| match op.r#type() {
                    install_operation::Type::ReplaceXz => Some("xz"),
                    install_operation::Type::ReplaceBz => Some("bz2"),
                    install_operation::Type::Zstd => Some("zstd"),
                    _ => None,
                })
                .unwrap_or("none")
                .to_string();
            let encryption = if partition.partition_name.contains("userdata") {
                "AES"
            } else {
                "none"
            };

            partitions.push(PartitionMetadata {
                partition_name: partition.partition_name.clone(),
                size_in_blocks,
                size_in_bytes,
                size_readable: format_size(size_in_bytes),
                hash,
                start_offset,
                end_offset,
                data_offset,
                partition_type: partition.partition_name.clone(),
                operations_count: partition.operations.len(),
                compression_type,
                encryption: encryption.to_string(),
                block_size,
                total_blocks,
                run_postinstall: partition.run_postinstall.clone(),
                postinstall_path: partition.postinstall_path.clone(),
                filesystem_type: partition.filesystem_type.clone(),
                postinstall_optional: partition.postinstall_optional.clone(),
                hash_tree_algorithm: partition.hash_tree_algorithm.clone(),
                version: partition.version.clone(),
            });
        }
    }

    // Convert dynamic partition metadata if available
    let dynamic_partition_metadata = if let Some(dpm) = &manifest.dynamic_partition_metadata {
        let groups: Vec<DynamicPartitionGroupInfo> = dpm
            .groups
            .iter()
            .map(|group| DynamicPartitionGroupInfo {
                name: group.name.clone(),
                size: group.size,
                partition_names: group.partition_names.clone(),
            })
            .collect();

        let vabc_feature_set = dpm.vabc_feature_set.as_ref().map(|fs| VabcFeatureSetInfo {
            threaded: fs.threaded,
            batch_writes: fs.batch_writes,
        });

        Some(DynamicPartitionInfo {
            groups,
            snapshot_enabled: dpm.snapshot_enabled,
            vabc_enabled: dpm.vabc_enabled,
            vabc_compression_param: dpm.vabc_compression_param.clone(),
            cow_version: dpm.cow_version,
            vabc_feature_set,
            compression_factor: dpm.compression_factor,
        })
    } else {
        None
    };

    // Convert APEX info
    let apex_info: Vec<ApexInfoMetadata> = manifest
        .apex_info
        .iter()
        .map(|info| ApexInfoMetadata {
            package_name: info.package_name.clone(),
            version: info.version,
            is_compressed: info.is_compressed,
            decompressed_size: info.decompressed_size,
        })
        .collect();

    let payload_metadata = PayloadMetadata {
        security_patch_level: manifest.security_patch_level.clone(),
        block_size: manifest.block_size.unwrap_or(4096),
        minor_version: manifest.minor_version.unwrap_or(0),
        max_timestamp: manifest.max_timestamp,
        dynamic_partition_metadata,
        partial_update: manifest.partial_update,
        apex_info,
        partitions,
    };

    let json = serde_json::to_string_pretty(&payload_metadata)?;
    let metadata_path = output_dir.join("payload_metadata.json");
    fs::write(metadata_path, json)?;

    Ok(())
}
fn format_elapsed_time(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let millis = duration.subsec_millis();

    if hours > 0 {
        format!("{}h {}m {}.{:03}s", hours, mins, secs, millis)
    } else if mins > 0 {
        format!("{}m {}.{:03}s", mins, secs, millis)
    } else {
        format!("{}.{:03}s", secs, millis)
    }
}
fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()?;
    }

    let start_time = Instant::now();

    let multi_progress = MultiProgress::new();
    let main_pb = multi_progress.add(ProgressBar::new_spinner());
    main_pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    main_pb.enable_steady_tick(Duration::from_millis(100));
    let payload_path_str = args.payload_path.to_string_lossy();
    let is_url =
        payload_path_str.starts_with("http://") || payload_path_str.starts_with("https://");
    main_pb.set_message("Opening file...");
    let mut payload_reader: Box<dyn ReadSeek> = if is_url {
        main_pb.set_message("Initializing remote connection...");
        let url = payload_path_str.to_string();

        // First check if it's a ZIP file
        let is_zip = url.ends_with(".zip");

        let content_type = if !is_zip {
            let content_type = HttpReader::new_silent(url.clone())
                .map(|r| r.content_type)
                .unwrap_or(None);
            content_type
        } else {
            None
        };

        if is_zip || content_type.as_deref() == Some("application/zip") {
            Box::new(RemoteZipReader::new(url)?) as Box<dyn ReadSeek>
        } else {
            Box::new(HttpReader::new(url)?) as Box<dyn ReadSeek>
        }
    } else if args.payload_path.extension().and_then(|e| e.to_str()) == Some("zip") {
        let path_str = args
            .payload_path
            .to_str()
            .ok_or_else(|| anyhow!("Invalid path"))?;
        let mut error = 0;
        let archive = unsafe { zip_open(path_str.as_ptr() as *const c_char, 0, &mut error) };
        if archive.is_null() {
            return Err(anyhow!("Failed to open ZIP file: error {}", error));
        }
        Box::new(LibZipReader::new(archive, path_str.to_string())?) as Box<dyn ReadSeek>
    } else {
        Box::new(File::open(&args.payload_path)?) as Box<dyn ReadSeek>
    };

    fs::create_dir_all(&args.out)?;
    let mut magic = [0u8; 4];
    payload_reader.read_exact(&mut magic)?;
    if magic != *b"CrAU" {
        return Err(anyhow!("Invalid payload file: magic 'CrAU' not found"));
    }
    let file_format_version = payload_reader.read_u64::<BigEndian>()?;
    if file_format_version != 2 {
        return Err(anyhow!(
            "Unsupported payload version: {}",
            file_format_version
        ));
    }
    let manifest_size = payload_reader.read_u64::<BigEndian>()?;
    let metadata_signature_size = payload_reader.read_u32::<BigEndian>()?;
    let mut manifest = vec![0u8; manifest_size as usize];
    payload_reader.read_exact(&mut manifest)?;
    let mut metadata_signature = vec![0u8; metadata_signature_size as usize];
    payload_reader.read_exact(&mut metadata_signature)?;
    let data_offset = payload_reader.stream_position()?;
    let manifest = DeltaArchiveManifest::decode(&manifest[..])?;

    // Display security patch level if available
    if let Some(security_patch) = &manifest.security_patch_level {
        println!("- Security Patch: {}", security_patch);
    }
    if args.metadata {
        main_pb.set_message("Extracting metadata...");
        if let Err(e) = save_metadata(&manifest, &args.out, data_offset) {
            main_pb.finish_with_message("✕ Failed to save metadata");
            eprintln!("Error saving metadata: {}", e);
            multi_progress.clear()?;
            return Err(e);
        } else {
           // main_pb.finish_with_message("✓ Metadata extraction complete");
            println!(
                "✓ Metadata saved at: {}/payload_metadata.json",
                args.out.display()
            );
            multi_progress.clear()?;
            return Ok(());
        }
    }
    if args.list {
        main_pb.finish_and_clear();
        payload_reader.seek(SeekFrom::Start(0))?;
        if let Err(e) = save_metadata(&manifest, &args.out, data_offset) {
            eprintln!("✕ Failed to save metadata: {}", e);
        } else {
            println!(
                "✓ Metadata saved at: {}/payload_metadata.json",
                args.out.display()
            );
        }
        return list_partitions(&mut payload_reader);
    }

    let block_size = manifest.block_size.unwrap_or(4096);
    let partitions_to_extract: Vec<_> = if args.images.is_empty() {
        manifest.partitions.iter().collect()
    } else {
        let images = args.images.split(',').collect::<HashSet<_>>();
        manifest
            .partitions
            .iter()
            .filter(|p| images.contains(p.partition_name.as_str()))
            .collect()
    };
    if partitions_to_extract.is_empty() {
        main_pb.finish_with_message("No partitions to extract");
        multi_progress.clear()?;
        return Ok(());
    }
    main_pb.set_message(format!(
        "Found {} partitions to extract",
        partitions_to_extract.len()
    ));
    if args.decompress_mode == DecompressMode::RawOnly {
        let raw_pb = multi_progress.add(ProgressBar::new_spinner());
        raw_pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        raw_pb.set_message("Extracting raw payload.bin...");
        payload_reader.seek(SeekFrom::Start(0))?;
        let mut out_file = File::create(args.out.join("payload.bin"))?;
        io::copy(&mut payload_reader, &mut out_file)?;
        raw_pb.finish_with_message("Raw payload extraction complete");
        main_pb.finish();
        multi_progress.clear()?;
        return Ok(());
    }
    let use_parallel = (!is_url
        && (args.payload_path.extension().and_then(|e| e.to_str()) == Some("zip")
            || args.payload_path.extension().and_then(|e| e.to_str()) == Some("bin")))
        && !args.no_parallel;
    main_pb.set_message(if use_parallel {
        "Extracting Partitions..."
    } else {
        "Processing partitions..."
    });
    let multi_progress = Arc::new(multi_progress);
    let args = Arc::new(args);
    if use_parallel {
        let payload_path = Arc::new(args.payload_path.to_str().unwrap().to_string());
        let max_retries = 3;
        let mut failed_partitions = Vec::new();
        let num_cpus = num_cpus::get();
        let chunk_size = std::cmp::max(1, partitions_to_extract.len() / num_cpus);
        let results: Vec<_> = partitions_to_extract
            .par_chunks(chunk_size)
            .flat_map(|chunk| {
                chunk.par_iter().map(|partition| {
                    let result = (0..max_retries)
                        .find_map(|attempt| {
                            if attempt > 0 {
                                std::thread::sleep(Duration::from_millis(100 * attempt as u64));
                            }
                            let mut reader = if args
                                .payload_path
                                .extension()
                                .and_then(|e| e.to_str())
                                == Some("zip")
                            {
                                Box::new(
                                    match LibZipReader::new_for_parallel((*payload_path).clone()) {
                                        Ok(reader) => reader,
                                        Err(e) => {
                                            return Some(Err((
                                                partition.partition_name.clone(),
                                                e,
                                            )));
                                        }
                                    },
                                ) as Box<dyn ReadSeek>
                            } else {
                                match create_payload_reader(&args.payload_path) {
                                    Ok(reader) => reader,
                                    Err(e) => {
                                        return Some(Err((partition.partition_name.clone(), e)));
                                    }
                                }
                            };
                            match dump_partition(
                                partition,
                                data_offset,
                                block_size as u64,
                                &args,
                                &mut reader,
                                Some(&multi_progress),
                            ) {
                                Ok(()) => Some(Ok(())),
                                Err(e) => {
                                    if attempt == max_retries - 1 {
                                        Some(Err((partition.partition_name.clone(), e)))
                                    } else {
                                        None
                                    }
                                }
                            }
                        })
                        .unwrap_or_else(|| {
                            Err((
                                partition.partition_name.clone(),
                                anyhow!("All retry attempts failed"),
                            ))
                        });

                    result
                })
            })
            .collect();
        for result in results {
            if let Err((partition_name, error)) = result {
                eprintln!("Failed to process partition {}: {}", partition_name, error);
                failed_partitions.push(partition_name);
            }
        }
        if !failed_partitions.is_empty() {
            main_pb.set_message(format!(
                "Retrying {} failed partitions sequentially...",
                failed_partitions.len()
            ));

            for partition in partitions_to_extract
                .iter()
                .filter(|p| failed_partitions.contains(&p.partition_name))
            {
                if let Err(e) = dump_partition(
                    partition,
                    data_offset,
                    block_size as u64,
                    &args,
                    &mut payload_reader,
                    Some(&multi_progress),
                ) {
                    eprintln!(
                        "Failed to process partition {} in sequential mode: {}",
                        partition.partition_name, e
                    );
                }
            }
        }

        let elapsed_time = format_elapsed_time(start_time.elapsed());

        if failed_partitions.is_empty() {
            main_pb.finish_with_message(format!(
                "Partitions Processed Successfully! (in {})",
                elapsed_time
            ));
            println!(
                "\nExtraction completed in {}. Check the output directory: {:?}",
                elapsed_time, args.out
            );
        } else {
            main_pb.finish_with_message(format!(
                "Completed with {} failed partitions. (in {})",
                failed_partitions.len(),
                elapsed_time
            ));
            println!(
                "\nExtraction completed with errors in {}. Check the output directory: {:?}",
                elapsed_time, args.out
            );
        }
    } else {
        let mut success_or_what = true;
        for partition in partitions_to_extract {
            if let Err(e) = dump_partition(
                partition,
                data_offset,
                block_size as u64,
                &args,
                &mut payload_reader,
                Some(&multi_progress),
            ) {
                eprintln!(
                    "Failed to process partition {}: {}",
                    partition.partition_name, e
                );
                success_or_what = false;
            }
        }

        let elapsed_time = format_elapsed_time(start_time.elapsed());

        if success_or_what {
            main_pb.finish_with_message(format!(
                "Partitions Processed Successfully! (in {})",
                elapsed_time
            ));
            println!(
                "\nExtraction completed in {}. Check the output directory: {:?}",
                elapsed_time, args.out
            );
        } else {
            main_pb.finish_with_message(format!(
                "Partition processing completed with errors. (in {})",
                elapsed_time
            ));
            println!(
                "\nExtraction completed with errors in {}. Check the output directory: {:?}",
                elapsed_time, args.out
            );
        }
    }
    Ok(())
}
