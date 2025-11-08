use crate::args::Args;
use crate::utils::format_size;
use crate::PartitionUpdate;
use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

const BUFFER_SIZE: usize = 1024 * 1024; // 1MB buffer

pub async fn verify_partitions_hash(
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
    verification_pb.enable_steady_tick(tokio::time::Duration::from_millis(100));
    verification_pb.set_message(format!(
        "Verifying hashes for {} partitions",
        partitions.len()
    ));

    let out_dir = &args.out;
    let mut failed_verifications = Vec::new();
    
    // Create progress bars for each partition
    let progress_bars: Vec<_> = partitions
        .iter()
        .map(|partition| {
            let pb = multi_progress.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} {msg}")
                    .unwrap(),
            );
            pb.enable_steady_tick(tokio::time::Duration::from_millis(100));
            pb.set_message(format!("Queuing {}", partition.partition_name));
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

                pb.set_message(format!("Verifying {}", partition_name));

                match verify_partition_hash(&partition_name, &out_path, expected_hash, Some(pb)).await {
                    Ok(true) => Ok(partition_name),
                    Ok(false) => Err(partition_name),
                    Err(e) => {
                        eprintln!("Error verifying hash for {}: {}", partition_name, e);
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
                eprintln!("Verification task panicked: {}", e);
            }
            _ => {}
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

async fn verify_partition_hash(
    partition_name: &str,
    out_path: &PathBuf,
    expected_hash: Option<&Vec<u8>>,
    progress_bar: Option<ProgressBar>,
) -> Result<bool> {
    let Some(expected) = expected_hash else {
        if let Some(pb) = progress_bar {
            pb.finish_with_message(format!("No hash for {}", partition_name));
        }
        return Ok(true);
    };

    if expected.is_empty() {
        if let Some(pb) = progress_bar {
            pb.finish_with_message(format!("No hash for {}", partition_name));
        }
        return Ok(true);
    }

    let mut file = File::open(out_path).await
        .with_context(|| format!("Failed to open {} for hash verification", partition_name))?;

    let file_size = file.metadata().await.map(|m| m.len()).unwrap_or(0);

    if let Some(pb) = &progress_bar {
        pb.set_message(format!(
            "Verifying {} ({})",
            partition_name,
            format_size(file_size)
        ));
    }

    // Hash the file
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
    let matches = hash.as_slice() == expected.as_slice();

    if let Some(pb) = progress_bar {
        if matches {
            pb.finish_with_message(format!("✓ {} verified", partition_name));
        } else {
            pb.finish_with_message(format!("✗ {} mismatch", partition_name));
        }
    }

    Ok(matches)
}