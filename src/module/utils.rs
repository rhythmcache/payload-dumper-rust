use crate::DeltaArchiveManifest;
use crate::ReadSeek;
use crate::install_operation;
use anyhow::{Result, anyhow};
use byteorder::{BigEndian, ReadBytesExt};
use prost::Message;
use std::io::SeekFrom;
use std::time::Duration;
/*
#[cfg(all(windows, feature = "local_zip"))]
use std::ffi::OsStr;
#[cfg(all(windows, feature = "local_zip"))]
use std::os::windows::ffi::OsStrExt;


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

*/

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

pub fn list_partitions(payload_reader: &mut Box<dyn ReadSeek>) -> Result<()> {
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
