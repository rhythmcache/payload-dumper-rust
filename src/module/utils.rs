use anyhow::{Context, Result, anyhow};
use byteorder::{BigEndian, ReadBytesExt};
use digest::Digest;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use prost::Message;
use rayon::prelude::*;
use serde_json;
use sha2::Sha256;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

use crate::DeltaArchiveManifest;
use crate::PartitionUpdate;
use crate::ReadSeek;
use crate::install_operation;
use crate::module::structs::{
    ApexInfoMetadata, Args, DynamicPartitionGroupInfo, DynamicPartitionInfo, PartitionMetadata,
    PayloadMetadata, VabcFeatureSetInfo,
};

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

pub fn verify_hash(data: &[u8], expected_hash: &[u8]) -> bool {
    if expected_hash.is_empty() {
        return true;
    }
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();

    hash.as_slice() == expected_hash
}

pub fn verify_partitions_hash(
    partitions: &[&PartitionUpdate],
    args: &Args,
    multi_progress: &MultiProgress,
) -> Result<Vec<String>> {
    if args.no_verify {
        return Ok(vec![]);
    }

    let verification_pb = multi_progress.add(ProgressBar::new_spinner());
    verification_pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    verification_pb.enable_steady_tick(Duration::from_millis(100));
    verification_pb.set_message(format!(
        "Verifying hashes for {} partitions",
        partitions.len()
    ));

    let out_dir = &args.out;
    let mut failed_verifications = Vec::new();
    let progress_bars: Vec<_> = partitions
        .iter()
        .map(|partition| {
            let pb = multi_progress.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} {msg}")
                    .unwrap(),
            );
            pb.enable_steady_tick(Duration::from_millis(100));
            pb.set_message(format!("Queuing {}", partition.partition_name));
            (partition.partition_name.clone(), pb)
        })
        .collect();

    let results: Vec<_> = partitions
        .par_iter()
        .map(|partition| {
            let partition_name = &partition.partition_name;
            let out_path = out_dir.join(format!("{}.img", partition_name));

            let expected_hash = partition
                .new_partition_info
                .as_ref()
                .and_then(|info| info.hash.as_ref());

            let pb = progress_bars
                .iter()
                .find(|(name, _)| name == partition_name)
                .map(|(_, pb)| pb.clone());

            if let Some(pb) = &pb {
                pb.set_message(format!("Verifying {}", partition_name));
            }

            let result = verify_partition_hash(partition_name, &out_path, expected_hash, pb);

            match result {
                Ok(true) => Ok(partition_name.clone()),
                Ok(false) => Err(partition_name.clone()),
                Err(e) => {
                    eprintln!("Error verifying hash for {}: {}", partition_name, e);
                    Err(partition_name.clone())
                }
            }
        })
        .collect();

    for result in results {
        if let Err(partition_name) = result {
            failed_verifications.push(partition_name);
        }
    }

    if failed_verifications.is_empty() {
        verification_pb.finish_with_message("All hashes verified successfully");
    } else {
        verification_pb.finish_with_message(format!(
            "Hash verification completed with {} failures",
            failed_verifications.len()
        ));
    }

    Ok(failed_verifications)
}

pub fn verify_partition_hash(
    partition_name: &str,
    out_path: &PathBuf,
    expected_hash: Option<&Vec<u8>>,
    progress_bar: Option<ProgressBar>,
) -> Result<bool> {
    if let Some(expected) = expected_hash {
        if expected.is_empty() {
            if let Some(pb) = progress_bar {
                pb.finish_with_message(format!("No hash for {}", partition_name));
            }
            return Ok(true);
        }

        let file = File::open(out_path)
            .with_context(|| format!("Failed to open {} for hash verification", partition_name))?;

        let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);

        if let Some(pb) = &progress_bar {
            pb.set_message(format!(
                "Verifying {} ({})",
                partition_name,
                format_size(file_size)
            ));
        }

        let mut hasher = Sha256::new();

        if file_size > 10 * 1024 * 1024 {
            match unsafe { memmap2::Mmap::map(&file) } {
                Ok(mmap) => {
                    hasher.update(&mmap[..]);

                    let hash = hasher.finalize();
                    let matches = hash.as_slice() == expected.as_slice();

                    if let Some(pb) = progress_bar {
                        if matches {
                            pb.finish_with_message(format!("✓ {} verified", partition_name));
                        } else {
                            pb.finish_with_message(format!("✕ {} mismatch", partition_name));
                        }
                    }

                    return Ok(matches);
                }
                Err(_) => {
                    // Fall back
                }
            }
        }

        let buffer_size = if file_size < 1024 * 1024 {
            64 * 1024
        } else if file_size < 100 * 1024 * 1024 {
            1024 * 1024
        } else {
            8 * 1024 * 1024
        };

        let mut file = std::io::BufReader::new(file);
        let mut buffer = vec![0u8; buffer_size];

        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }

            hasher.update(&buffer[..bytes_read]);
        }

        let hash = hasher.finalize();
        let matches = hash.as_slice() == expected.as_slice();

        if let Some(pb) = progress_bar {
            if matches {
                pb.finish_with_message(format!("✓ {} verified", partition_name));
            } else {
                pb.finish_with_message(format!("✕ {} mismatch", partition_name));
            }
        }

        Ok(matches)
    } else {
        if let Some(pb) = progress_bar {
            pb.finish_with_message(format!("No hash for {}", partition_name));
        }
        Ok(true)
    }
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

pub fn save_metadata(
    manifest: &DeltaArchiveManifest,
    output_dir: &PathBuf,
    data_offset: u64,
) -> Result<String> {
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

    if output_dir.to_string_lossy() == "-" {
        return Ok(json);
    }

    let metadata_path = output_dir.join("payload_metadata.json");
    fs::write(metadata_path, &json)?;

    Ok(json)
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
