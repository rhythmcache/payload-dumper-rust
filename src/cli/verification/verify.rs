// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use crate::cli::args::args_def::Args;
use crate::cli::ui::ui_print::UiOutput;
use anyhow::{Context, Result};
use payload_dumper::structs::PartitionUpdate;
use payload_dumper::utils::format_size;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt;

const BUFFER_SIZE: usize = 1024 * 1024; // 1MB buffer

/// result status of a hash verification check
enum HashVerificationStatus {
    Verified,
    Mismatch,
    NoHash,
}

pub async fn verify_partitions_hash(
    partitions: &[&PartitionUpdate],
    args: &Args,
    ui: &UiOutput,
) -> Result<Vec<String>> {
    if args.no_verify {
        return Ok(vec![]);
    }

    let verification_pb = ui.create_spinner(format!(
        "Verifying hashes for {} partitions",
        partitions.len()
    ));

    let out_dir = &args.out;
    let mut failed_verifications = Vec::new();

    let progress_bars: Vec<_> = partitions
        .iter()
        .map(|partition| {
            let pb = ui.create_spinner(format!("Queuing {}", partition.partition_name));
            (partition.partition_name.clone(), pb)
        })
        .collect();

    // Process partitions in parallel
    let tasks: Vec<_> = partitions
        .iter()
        .enumerate()
        .map(|(idx, partition)| {
            let partition = (*partition).clone();
            let out_dir = out_dir.clone();
            let pb = progress_bars[idx].1.clone();

            tokio::spawn(async move {
                let partition_name = partition.partition_name.clone();
                let out_path = out_dir.join(format!("{}.img", partition_name));

                let expected_hash = partition
                    .new_partition_info
                    .as_ref()
                    .and_then(|info| info.hash.as_ref());

                if let Some(p) = &pb {
                    let size_str = match fs::metadata(&out_path).await {
                        Ok(m) => format_size(m.len()),
                        Err(_) => "unknown size".to_string(),
                    };
                    p.set_message(format!("Verifying {} ({})", partition_name, size_str));
                }

                // Perform Logic (Pure)
                let result = verify_partition_file(&out_path, expected_hash).await;

                // Update UI: Result
                match result {
                    Ok(HashVerificationStatus::Verified) => {
                        if let Some(p) = &pb {
                            p.finish_with_message(format!("✓ {} verified", partition_name));
                        }
                        Ok(partition_name)
                    }
                    Ok(HashVerificationStatus::Mismatch) => {
                        if let Some(p) = &pb {
                            p.finish_with_message(format!("✗ {} mismatch", partition_name));
                        }
                        Err(partition_name)
                    }
                    Ok(HashVerificationStatus::NoHash) => {
                        if let Some(p) = &pb {
                            p.finish_with_message(format!("No hash for {}", partition_name));
                        }
                        Ok(partition_name)
                    }
                    Err(e) => {
                        let msg = format!("Error verifying hash for {}: {}", partition_name, e);
                        if let Some(p) = &pb {
                            p.println(msg); // Print above bar to avoid tearing
                            p.finish_with_message(format!("✗ {} error", partition_name));
                        } else {
                            eprintln!("{}", msg);
                        }
                        Err(partition_name)
                    }
                }
            })
        })
        .collect();

    // Wait for all verification tasks
    let results = futures::future::join_all(tasks).await;

    for result in results {
        match result {
            Ok(Err(partition_name)) => {
                failed_verifications.push(partition_name);
            }
            Err(e) => {
                ui.pb_eprintln(format!("Verification task panicked: {}", e));
            }
            _ => {}
        }
    }

    if failed_verifications.is_empty() {
        if let Some(pb) = verification_pb {
            pb.finish_with_message("All hashes verified successfully");
        }
    } else if let Some(pb) = verification_pb {
        pb.finish_with_message(format!(
            "Hash verification completed with {} failures",
            failed_verifications.len()
        ));
    }

    Ok(failed_verifications)
}

async fn verify_partition_file(
    out_path: &Path,
    expected_hash: Option<&Vec<u8>>,
) -> Result<HashVerificationStatus> {
    let Some(expected) = expected_hash else {
        return Ok(HashVerificationStatus::NoHash);
    };

    if expected.is_empty() {
        return Ok(HashVerificationStatus::NoHash);
    }

    let mut file = File::open(out_path)
        .await
        .with_context(|| format!("Failed to open {:?} for hash verification", out_path))?;

    // hash the file
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_SIZE];

    loop {
        let bytes_read = file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize().to_vec();
    if hash.as_slice() == expected.as_slice() {
        Ok(HashVerificationStatus::Verified)
    } else {
        Ok(HashVerificationStatus::Mismatch)
    }
}
