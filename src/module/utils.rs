use crate::DeltaArchiveManifest;
use crate::ReadSeek;
use crate::install_operation;
use anyhow::{Result, anyhow};
use byteorder::{BigEndian, ReadBytesExt};
use prost::Message;
use std::io::SeekFrom;
use std::time::Duration;

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
