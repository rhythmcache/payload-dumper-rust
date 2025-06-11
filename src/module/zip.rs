use crate::module::utils::{get_zip_error_message, handle_path};
use anyhow::{Result, anyhow};
use memmap2::Mmap;
use std::ffi::CStr;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::raw::{c_char, c_int, c_void};

#[repr(C)]
pub struct zip_stat_t {
    valid: u64,
    name: *const c_char,
    index: u64,
    size: u64,
}

#[repr(C)]
pub struct zip_error_t {
    zip_err: c_int,
    sys_err: c_int,
    str_: *mut c_char,
}

pub struct LibZipReader {
    archive: *mut c_void,
    file: *mut c_void,
    current_position: u64,
    file_size: u64,
    mmap: Option<Mmap>,
    buffer: Vec<u8>,
    buffer_size: usize,
    cached_filename: Option<std::ffi::CString>,
    file_index: i64,
    is_seekable: Option<bool>,
}

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
    pub fn zip_fseek(file: *mut c_void, offset: i64, whence: c_int) -> i8;
    pub fn zip_file_is_seekable(file: *mut c_void) -> c_int;
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

impl Default for zip_error_t {
    fn default() -> Self {
        zip_error_t {
            zip_err: 0,
            sys_err: 0,
            str_: std::ptr::null_mut(),
        }
    }
}

impl LibZipReader {
    pub fn new(archive: *mut c_void, _path: String) -> Result<Self> {
        unsafe {
            let payload_name = match CStr::from_bytes_with_nul(b"payload.bin\0") {
                Ok(name) => name,
                Err(_) => return Err(anyhow!("Failed to create CStr for payload.bin")),
            };

            let mut stat = zip_stat_t::default();
            let mut file_size = 0;
            let mut file = std::ptr::null_mut();
            let mut file_index = -1;
            if zip_stat(archive, payload_name.as_ptr(), 0, &mut stat) == 0 {
                file = zip_fopen(archive, payload_name.as_ptr(), 0);
                if !file.is_null() {
                    file_size = stat.size;
                }
            }

            if file.is_null() {
                let num_entries = zip_get_num_entries(archive, 0);

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
                    return Err(anyhow!("payload.bin not found in ZIP file"));
                }

                if zip_stat_index(archive, file_index as u64, 0, &mut stat) != 0 {
                    return Err(anyhow!("Failed to get file stats"));
                }

                file = zip_fopen_index(archive, file_index as u64, 0);
                if file.is_null() {
                    return Err(anyhow!("Failed to open payload.bin in ZIP"));
                }

                file_size = stat.size;
            }

            if file_size == 0 {
                zip_fclose(file);
                return Err(anyhow!("payload.bin has zero size"));
            }

            let cached_filename = if file_index != -1 {
                None
            } else {
                std::ffi::CString::new("payload.bin").ok()
            };

            let buffer_size = 8 * 1024 * 1024;
            let buffer = vec![0u8; buffer_size];
            let mmap = None;

            Ok(Self {
                archive,
                file,
                current_position: 0,
                file_size,
                mmap,
                buffer,
                buffer_size,
                cached_filename,
                file_index,
                is_seekable: None,
            })
        }
    }

    pub fn new_for_parallel(path: String) -> Result<Self> {
        unsafe {
            let normalized_path = handle_path(&path)?;

            let c_path = match std::ffi::CString::new(normalized_path.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return Err(anyhow!("Invalid path contains null bytes: {}", e));
                }
            };

            let mut error_code: c_int = 0;
            let archive = zip_open(c_path.as_ptr(), 0, &mut error_code);

            if archive.is_null() {
                let error_msg = get_zip_error_message(error_code);
                return Err(anyhow!(
                    "Failed to open ZIP file: {} (error code: {})",
                    error_msg,
                    error_code
                ));
            }

            match Self::new(archive, path) {
                Ok(reader) => Ok(reader),
                Err(e) => {
                    zip_close(archive);
                    Err(e)
                }
            }
        }
    }

    // Check if the file is seekable
    fn check_seekable(&mut self) -> io::Result<bool> {
        if let Some(seekable) = self.is_seekable {
            return Ok(seekable);
        }

        unsafe {
            if self.file.is_null() {
                return Err(io::Error::new(io::ErrorKind::Other, "File handle is null"));
            }

            let result = zip_file_is_seekable(self.file);
            match result {
                1 => {
                    self.is_seekable = Some(true);
                    Ok(true)
                }
                0 => {
                    self.is_seekable = Some(false);
                    Ok(false)
                }
                -1 => Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Invalid argument when checking seekability",
                )),
                _ => Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Unexpected return value from zip_file_is_seekable",
                )),
            }
        }
    }

    fn fast_seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
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

        let (offset, whence) = match pos {
            SeekFrom::Start(offset) => (offset as i64, 0), // SEEK_SET
            SeekFrom::Current(offset) => (offset, 1),      // SEEK_CUR
            SeekFrom::End(offset) => (offset, 2),          // SEEK_END
        };

        unsafe {
            if self.file.is_null() {
                return Err(io::Error::new(io::ErrorKind::Other, "File handle is null"));
            }

            let result = zip_fseek(self.file, offset, whence);
            if result != 0 {
                return Err(io::Error::new(io::ErrorKind::Other, "zip_fseek failed"));
            }

            self.current_position = new_pos;
            Ok(self.current_position)
        }
    }

    fn slow_seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
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

        if new_pos == self.current_position {
            return Ok(new_pos);
        }

        // Small forward seeks can be done by reading and discarding
        if new_pos > self.current_position && new_pos - self.current_position <= 8 * 1024 {
            let mut skip_buf = vec![0u8; (new_pos - self.current_position) as usize];
            self.read_exact(&mut skip_buf)?;
            return Ok(self.current_position);
        }

        // For larger seeks, reopen the file and read to the position
        unsafe {
            if !self.file.is_null() {
                zip_fclose(self.file);
                self.file = std::ptr::null_mut();
            }

            if self.file_index >= 0 {
                self.file = zip_fopen_index(self.archive, self.file_index as u64, 0);
            } else if let Some(ref filename) = self.cached_filename {
                self.file = zip_fopen(self.archive, filename.as_ptr(), 0);
            } else {
                // Fallback
                let payload_name = match CStr::from_bytes_with_nul(b"payload.bin\0") {
                    Ok(name) => name,
                    Err(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            "Failed to create CStr for payload.bin",
                        ));
                    }
                };
                self.file = zip_fopen(self.archive, payload_name.as_ptr(), 0);
            }

            if self.file.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Failed to reopen file for seeking",
                ));
            }

            // Reset seekability cache since we reopened the file
            self.is_seekable = None;
            self.current_position = 0;

            if new_pos > 0 {
                if self.buffer.len() < self.buffer_size {
                    self.buffer.resize(self.buffer_size, 0);
                }

                let mut remaining = new_pos;
                while remaining > 0 {
                    let to_read = remaining.min(self.buffer.len() as u64) as usize;
                    let read_bytes =
                        zip_fread(self.file, self.buffer.as_mut_ptr() as *mut c_void, to_read);
                    if read_bytes <= 0 {
                        return Err(io::Error::new(io::ErrorKind::Other, "Failed to seek"));
                    }
                    self.current_position += read_bytes as u64;
                    remaining -= read_bytes as u64;

                    if read_bytes < to_read as isize {
                        break;
                    }
                }
            }

            Ok(self.current_position)
        }
    }
}

impl Drop for LibZipReader {
    fn drop(&mut self) {
        unsafe {
            if !self.file.is_null() {
                zip_fclose(self.file);
                self.file = std::ptr::null_mut();
            }

            if !self.archive.is_null() {
                let result = zip_close(self.archive);
                debug_assert_eq!(result, 0, "zip_close failed with code {}", result);
                self.archive = std::ptr::null_mut();
            }
        }
    }
}

impl Read for LibZipReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        if self.current_position >= self.file_size {
            return Ok(0);
        }

        if let Some(mmap) = &self.mmap {
            let start = self.current_position as usize;
            let end = (self.current_position + buf.len() as u64) as usize;

            if start >= mmap.len() {
                return Ok(0); // EOF
            }

            let end = end.min(mmap.len());
            let bytes_to_read = end - start;

            buf[..bytes_to_read].copy_from_slice(&mmap[start..end]);
            self.current_position += bytes_to_read as u64;

            return Ok(bytes_to_read);
        }

        let remaining = self.file_size - self.current_position;
        if remaining == 0 {
            return Ok(0);
        }

        let to_read = if (remaining as usize) < buf.len() {
            &mut buf[..remaining as usize]
        } else {
            buf
        };

        unsafe {
            if self.file.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Attempted to read from null file handle",
                ));
            }

            let read_bytes = zip_fread(
                self.file,
                to_read.as_mut_ptr() as *mut c_void,
                to_read.len(),
            );
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
        if self.mmap.is_some() {
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

            self.current_position = new_pos;
            return Ok(self.current_position);
        }

        // Check if file is seekable
        match self.check_seekable() {
            Ok(true) => self.fast_seek(pos),
            Ok(false) => self.slow_seek(pos),
            Err(e) => {
                // If we can't determine seekability, fall back to slow seek
                eprintln!(
                    "Warning: Could not determine file seekability, using slow seek: {}",
                    e
                );
                self.slow_seek(pos)
            }
        }
    }
}
