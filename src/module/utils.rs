use crate::DeltaArchiveManifest;
use crate::install_operation;
#[cfg(feature = "local_zip")]
use anyhow::Result;
#[cfg(all(windows, feature = "local_zip"))]
use std::ffi::OsStr;
#[cfg(all(windows, feature = "local_zip"))]
use std::os::windows::ffi::OsStrExt;
use std::time::Duration;

#[cfg(all(windows, feature = "local_zip"))]
pub fn handle_path(path: &str) -> Result<String> {
    let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
    let utf8_path = String::from_utf16_lossy(&wide[..wide.len() - 1]);
    Ok(utf8_path.replace('\\', "/"))
}

#[cfg(all(not(windows), feature = "local_zip"))]
pub fn handle_path(path: &str) -> Result<String> {
    Ok(path.to_string())
}

#[cfg(feature = "local_zip")]
pub fn get_zip_error_message(error_code: i32) -> &'static str {
    match error_code {
        0 => "No error",
        1 => "Multi-disk zip archives not supported",
        2 => "Renaming temporary file failed",
        3 => "Closing zip archive failed",
        4 => "Seek error",
        5 => "Read error",
        6 => "Write error",
        7 => "CRC error",
        8 => "Containing zip archive was closed",
        9 => "No such file",
        10 => "File already exists",
        11 => "Can't open file",
        12 => "Failure to create temporary file",
        13 => "Zlib error",
        14 => "Memory allocation failure",
        15 => "Entry has been changed",
        16 => "Compression method not supported",
        17 => "Premature end of file",
        18 => "Invalid argument",
        19 => "Not a zip archive",
        20 => "Internal error",
        21 => "Zip archive inconsistent",
        22 => "Can't remove file",
        23 => "Entry has been deleted",
        24 => "Encryption method not supported",
        25 => "Read-only archive",
        26 => "No password provided",
        27 => "Wrong password provided",
        28 => "Operation not supported",
        29 => "Resource still in use",
        30 => "Tell error",
        31 => "Compressed data invalid",
        _ => "Unknown error",
    }
}

pub fn is_differential_ota(manifest: &DeltaArchiveManifest) -> bool {
    manifest.partitions.iter().any(|partition| {
        partition.operations.iter().any(|op| {
            matches!(
                op.r#type(),
                install_operation::Type::SourceCopy
                    | install_operation::Type::SourceBsdiff
                    | install_operation::Type::BrotliBsdiff
            )
        })
    })
}

pub fn format_elapsed_time(duration: Duration) -> String {
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

pub fn format_size(size: u64) -> String {
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
