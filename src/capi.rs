// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use std::ffi::{CStr, CString, c_char, c_void};
use std::panic;
use std::ptr;
use std::sync::Arc;

use crate::extractor::local::{
    ExtractionProgress, ExtractionStatus, ProgressCallback, extract_partition,
    extract_partition_zip, list_partitions, list_partitions_zip,
};

/* Error Handling */

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = std::cell::RefCell::new(None);
}

fn set_last_error(err: String) {
    LAST_ERROR.with(|last| {
        *last.borrow_mut() = CString::new(err).ok();
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|last| {
        *last.borrow_mut() = None;
    });
}

/// get the last error message
/// returns NULL if no error occurred
/// the returned string is valid until the next call from the same thread
///
/// point: errors are thread-local. Each thread maintains its own error state.
#[unsafe(no_mangle)]
pub extern "C" fn payload_get_last_error() -> *const c_char {
    LAST_ERROR.with(|last| {
        last.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(ptr::null())
    })
}

/// clear the last error
#[unsafe(no_mangle)]
pub extern "C" fn payload_clear_error() {
    clear_last_error();
}

/* String Handling */

/// free a string allocated by this library
#[unsafe(no_mangle)]
pub extern "C" fn payload_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

/* Partition List API (payload.bin) */

/// list all partitions in a payload.bin file
/// Returns a JSON string on success, NULL on failure
/// the caller must free the returned string with payload_free_string()
///
/// the returned JSON structure:
/// {
///   "partitions": [...],
///   "total_partitions": 10,
///   "total_operations": 1000,
///   "total_size_bytes": 5000000000,
///   "total_size_readable": "4.66 GB"
/// }
#[unsafe(no_mangle)]
pub extern "C" fn payload_list_partitions(payload_path: *const c_char) -> *mut c_char {
    clear_last_error();

    let result = panic::catch_unwind(|| {
        if payload_path.is_null() {
            set_last_error("payload_path is NULL".to_string());
            return ptr::null_mut();
        }

        let path_str = unsafe {
            match CStr::from_ptr(payload_path).to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!("Invalid UTF-8 in payload_path: {}", e));
                    return ptr::null_mut();
                }
            }
        };

        match list_partitions(path_str) {
            Ok(json) => match CString::new(json) {
                Ok(c_str) => c_str.into_raw(),
                Err(e) => {
                    set_last_error(format!("Failed to create C string: {}", e));
                    ptr::null_mut()
                }
            },
            Err(e) => {
                set_last_error(format!("Failed to list partitions: {}", e));
                ptr::null_mut()
            }
        }
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("Panic occurred in payload_list_partitions".to_string());
            ptr::null_mut()
        }
    }
}

/* Partition List API ( Zip file ) */

/// list all partitions in a ZIP file containing payload.bin
/// returns a JSON string on success, NULL on failure
/// the caller must free the returned string with payload_free_string()
///
/// the returned JSON format is the same as payload_list_partitions()
#[unsafe(no_mangle)]
pub extern "C" fn payload_list_partitions_zip(zip_path: *const c_char) -> *mut c_char {
    clear_last_error();

    let result = panic::catch_unwind(|| {
        if zip_path.is_null() {
            set_last_error("zip_path is NULL".to_string());
            return ptr::null_mut();
        }

        let path_str = unsafe {
            match CStr::from_ptr(zip_path).to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!("Invalid UTF-8 in zip_path: {}", e));
                    return ptr::null_mut();
                }
            }
        };

        match list_partitions_zip(path_str) {
            Ok(json) => match CString::new(json) {
                Ok(c_str) => c_str.into_raw(),
                Err(e) => {
                    set_last_error(format!("Failed to create C string: {}", e));
                    ptr::null_mut()
                }
            },
            Err(e) => {
                set_last_error(format!("Failed to list partitions from ZIP: {}", e));
                ptr::null_mut()
            }
        }
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("Panic occurred in payload_list_partitions_zip".to_string());
            ptr::null_mut()
        }
    }
}

/* Progress Callback */

/// progress callback function type
///
/// @param user_data User-provided data pointer
/// @param partition_name Name of the partition being extracted (temporary pointer)
/// @param current_operation Current operation number (0-based)
/// @param total_operations Total number of operations
/// @param percentage Completion percentage (0.0 to 100.0)
/// @param status Status code (see STATUS_* constants)
/// @param warning_message Warning message if status is STATUS_WARNING (temporary pointer)
/// @return non-zero to continue extraction, 0 to cancel
pub type CProgressCallback = extern "C" fn(
    user_data: *mut c_void,
    partition_name: *const c_char,
    current_operation: u64,
    total_operations: u64,
    percentage: f64,
    status: i32,
    warning_message: *const c_char,
) -> i32;

/// status codes for progress callback
pub const STATUS_STARTED: i32 = 0;
pub const STATUS_IN_PROGRESS: i32 = 1;
pub const STATUS_COMPLETED: i32 = 2;
pub const STATUS_WARNING: i32 = 3;

struct CCallbackWrapper {
    callback: CProgressCallback,
    user_data: *mut c_void,
}

// we require the user_data to be thread-safe
unsafe impl Send for CCallbackWrapper {}
unsafe impl Sync for CCallbackWrapper {}

impl CCallbackWrapper {
    fn call(&self, progress: ExtractionProgress) -> bool {
        // catch panics to prevent unwinding through C
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            // allocate partition name as local CString
            let partition_name = match CString::new(progress.partition_name.clone()) {
                Ok(s) => s,
                Err(_) => return true, // continue on error
            };

            // handle status and warning message
            // keep warning_msg as a local CString so it's automatically freed
            let warning_msg;
            let (status, warning_msg_ptr) = match progress.status {
                ExtractionStatus::Started => (STATUS_STARTED, ptr::null()),
                ExtractionStatus::InProgress => (STATUS_IN_PROGRESS, ptr::null()),
                ExtractionStatus::Completed => (STATUS_COMPLETED, ptr::null()),
                ExtractionStatus::Warning { message, .. } => {
                    // Create local CString - no into_raw(), no manual cleanup needed
                    warning_msg = CString::new(message).ok();
                    let msg_ptr = warning_msg
                        .as_ref()
                        .map(|s| s.as_ptr())
                        .unwrap_or(ptr::null());
                    (STATUS_WARNING, msg_ptr)
                }
            };

            // call the C callback
            // all pointers are valid for the duration of this call
            // they will be automatically freed when locals are dropped
            let result = (self.callback)(
                self.user_data,
                partition_name.as_ptr(),
                progress.current_operation,
                progress.total_operations,
                progress.percentage,
                status,
                warning_msg_ptr,
            );

            result != 0
            // partition_name and warning_msg are automatically dropped here
        }));

        // if callback panicked, log error and continue
        match result {
            Ok(should_continue) => should_continue,
            Err(_) => {
                eprintln!("WARNING: Progress callback panicked - continuing extraction");
                true // Continue on panic
            }
        }
    }
}

/* Extract Partition API (payload.bin) */

/// extract a single partition from a payload.bin file
///
/// @param payload_path Path to the payload.bin file
/// @param partition_name Name of the partition to extract
/// @param output_path Path where the partition image will be written
/// @param callback Optional progress callback (pass NULL for no callback)
/// @param user_data User data passed to callback (can be NULL)
/// @return 0 on success, -1 on failure (check payload_get_last_error())
///
/// This function can be safely called from multiple threads concurrently.
/// Each thread can extract a different partition in parallel.
///
/// - pass NULL for callback parameter if you don't want progress updates
/// - the partition_name and warning_message pointers passed to the callback
///   are ONLY valid during the callback execution. Do NOT store these pointers.
/// - If you need to keep the strings, copy them immediately in the callback.
/// - do NOT call free() on these strings, they are managed by the library.
///
/// - Return 0 from the callback to cancel extraction
/// - Return non-zero to continue
/// - cancellation may not be immediate
#[unsafe(no_mangle)]
pub extern "C" fn payload_extract_partition(
    payload_path: *const c_char,
    partition_name: *const c_char,
    output_path: *const c_char,
    callback: CProgressCallback, // function pointer (use NULL-equivalent cast from C side)
    user_data: *mut c_void,
) -> i32 {
    clear_last_error();

    let result = panic::catch_unwind(|| {
        // validate inputs
        if payload_path.is_null() {
            set_last_error("payload_path is NULL".to_string());
            return -1;
        }
        if partition_name.is_null() {
            set_last_error("partition_name is NULL".to_string());
            return -1;
        }
        if output_path.is_null() {
            set_last_error("output_path is NULL".to_string());
            return -1;
        }

        // convert C strings
        let payload_str = unsafe {
            match CStr::from_ptr(payload_path).to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!("Invalid UTF-8 in payload_path: {}", e));
                    return -1;
                }
            }
        };

        let partition_str = unsafe {
            match CStr::from_ptr(partition_name).to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!("Invalid UTF-8 in partition_name: {}", e));
                    return -1;
                }
            }
        };

        let output_str = unsafe {
            match CStr::from_ptr(output_path).to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!("Invalid UTF-8 in output_path: {}", e));
                    return -1;
                }
            }
        };

        // check if callback is null by comparing function pointer to null cast
        let progress_cb: Option<ProgressCallback> = if callback as usize == 0 {
            None
        } else {
            let wrapper = Arc::new(CCallbackWrapper {
                callback,
                user_data,
            });

            Some(Box::new(move |progress| wrapper.call(progress)) as ProgressCallback)
        };

        match extract_partition(payload_str, partition_str, output_str, progress_cb) {
            Ok(()) => 0,
            Err(e) => {
                set_last_error(format!("Extraction failed: {}", e));
                -1
            }
        }
    });

    match result {
        Ok(code) => code,
        Err(_) => {
            set_last_error("Panic occurred in payload_extract_partition".to_string());
            -1
        }
    }
}

/* Extract Partition API (ZIP file) */

/// extract a single partition from a ZIP file containing payload.bin
///
/// @param zip_path Path to the ZIP file containing payload.bin
/// @param partition_name Name of the partition to extract
/// @param output_path Path where the partition image will be written
/// @param callback Optional progress callback (pass NULL for no callback)
/// @param user_data User data passed to callback (can be NULL)
/// @return 0 on success, -1 on failure (check payload_get_last_error())
///
/// this function can be safely called from multiple threads concurrently.
/// each thread can extract a different partition in parallel.
///
/// - pass NULL for callback parameter if you don't want progress updates
/// - the partition_name and warning_message pointers passed to the callback
///   are ONLY valid during the callback execution. Do NOT store these pointers.
/// - if you need to keep the strings, copy them immediately in the callback.
/// - Do NOT call free() on these strings, they are managed by the library.
///
/// - Return 0 from the callback to cancel extraction
/// - Return non-zero to continue
/// - Cancellation may not be immediate
#[unsafe(no_mangle)]
pub extern "C" fn payload_extract_partition_zip(
    zip_path: *const c_char,
    partition_name: *const c_char,
    output_path: *const c_char,
    callback: CProgressCallback, // function pointer (use NULL-equivalent cast from C side)
    user_data: *mut c_void,
) -> i32 {
    clear_last_error();

    let result = panic::catch_unwind(|| {
        // validate inputs
        if zip_path.is_null() {
            set_last_error("zip_path is NULL".to_string());
            return -1;
        }
        if partition_name.is_null() {
            set_last_error("partition_name is NULL".to_string());
            return -1;
        }
        if output_path.is_null() {
            set_last_error("output_path is NULL".to_string());
            return -1;
        }

        let zip_str = unsafe {
            match CStr::from_ptr(zip_path).to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!("Invalid UTF-8 in zip_path: {}", e));
                    return -1;
                }
            }
        };

        let partition_str = unsafe {
            match CStr::from_ptr(partition_name).to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!("Invalid UTF-8 in partition_name: {}", e));
                    return -1;
                }
            }
        };

        let output_str = unsafe {
            match CStr::from_ptr(output_path).to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(format!("Invalid UTF-8 in output_path: {}", e));
                    return -1;
                }
            }
        };

        // check if callback is null by comparing function pointer to null cast
        let progress_cb: Option<ProgressCallback> = if callback as usize == 0 {
            None
        } else {
            let wrapper = Arc::new(CCallbackWrapper {
                callback,
                user_data,
            });

            Some(Box::new(move |progress| wrapper.call(progress)) as ProgressCallback)
        };

        match extract_partition_zip(zip_str, partition_str, output_str, progress_cb) {
            Ok(()) => 0,
            Err(e) => {
                set_last_error(format!("Extraction from ZIP failed: {}", e));
                -1
            }
        }
    });

    match result {
        Ok(code) => code,
        Err(_) => {
            set_last_error("Panic occurred in payload_extract_partition_zip".to_string());
            -1
        }
    }
}

/* Utility Functions */

/// get library version
/// returns a static string, do not free
#[unsafe(no_mangle)]
pub extern "C" fn payload_get_version() -> *const c_char {
    static C_VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();
    C_VERSION.as_ptr() as *const c_char
}

/// initialize the library (optional, but recommended for thread safety)
/// should be called once before any other library functions
/// @return 0 on success, -1 on failure
#[unsafe(no_mangle)]
pub extern "C" fn payload_init() -> i32 {
    // not yet implemented idk
    0
}

/// cleanup library resources
/// should be called once when done using the library
/// no library functions should be called after this
#[unsafe(no_mangle)]
pub extern "C" fn payload_cleanup() {
    // not yet implemented idk
}
