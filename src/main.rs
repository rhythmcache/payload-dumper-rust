use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow};

use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use prost::Message;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

mod args;
mod metadata;
mod payload_dumper;
mod utils;
mod verify;
mod structs;

use crate::args::Args;
#[cfg(feature = "metadata")]
use crate::metadata::save_metadata;
use crate::payload_dumper::{dump_partition, AsyncPayloadReader};
use crate::utils::{format_elapsed_time, format_size, is_differential_ota, list_partitions};
use crate::verify::verify_partitions_hash;

include!("proto/update_metadata.rs");

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // validate metadata feature usage
    #[cfg(not(feature = "metadata"))]
    if args.metadata.is_some() {
        return Err(anyhow!(
            "Metadata functionality requires the 'metadata' feature to be enabled. Please recompile with --features metadata"
        ));
    }

    let thread_count = if args.no_parallel {
        1
    } else if let Some(threads) = args.threads {
        threads
    } else {
        num_cpus::get()
    };

    println!("- Initialized {} thread(s)", thread_count);

    let start_time = Instant::now();

    let multi_progress = MultiProgress::new();
    let main_pb = multi_progress.add(ProgressBar::new_spinner());
    main_pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    main_pb.enable_steady_tick(tokio::time::Duration::from_millis(100));

    let payload_path_str = args.payload_path.to_string_lossy().to_string();

    // Check if we're outputting to stdout
    let is_stdout = args.out.to_string_lossy() == "-";

    // Check if it's a local .bin file
    let is_local_bin = args.payload_path.extension().and_then(|e| e.to_str()) == Some("bin")
        || args.payload_path.extension().and_then(|e| e.to_str()).is_none();

    if !is_local_bin {
        return Err(anyhow!(
            "Currently only local .bin payload files are supported in async mode"
        ));
    }

    main_pb.set_message("Opening file...");

    // Get file metadata
    if let Ok(metadata) = fs::metadata(&args.payload_path).await {
        if metadata.len() > 1024 * 1024 {
            if is_stdout {
                eprintln!(
                    "- Processing file: {}, size: {}",
                    payload_path_str,
                    format_size(metadata.len())
                );
            } else {
                println!(
                    "- Processing file: {}, size: {}",
                    payload_path_str,
                    format_size(metadata.len())
                );
            }
        }
    }

    // Open the payload file
    let mut payload_file = tokio::fs::File::open(&args.payload_path).await?;

    if args.out.to_string_lossy() != "-" {
        fs::create_dir_all(&args.out).await?;
    }

    // Read and validate magic header
    let mut magic = [0u8; 4];
    payload_file.read_exact(&mut magic).await?;
    if magic != *b"CrAU" {
        return Err(anyhow!("Invalid payload file: magic 'CrAU' not found"));
    }

    // Read header information
    let file_format_version = payload_file.read_u64().await?;
    if file_format_version != 2 {
        return Err(anyhow!(
            "Unsupported payload version: {}",
            file_format_version
        ));
    }

    let manifest_size = payload_file.read_u64().await?;
    let metadata_signature_size = payload_file.read_u32().await?;

    main_pb.set_message("Reading manifest...");

    // Read manifest
    let mut manifest = vec![0u8; manifest_size as usize];
    payload_file.read_exact(&mut manifest).await?;

    // Read metadata signature
    let mut metadata_signature = vec![0u8; metadata_signature_size as usize];
    payload_file.read_exact(&mut metadata_signature).await?;

    // Get data offset
    let data_offset = payload_file.stream_position().await?;

    // Decode manifest
    let manifest = DeltaArchiveManifest::decode(&manifest[..])?;

    // Check for differential OTA and abort if found
    if is_differential_ota(&manifest) {
        return Err(anyhow!(
            "This is a differential OTA package which is not supported. Please use a full OTA package instead."
        ));
    }

    // Print security patch level
    if let Some(security_patch) = &manifest.security_patch_level {
        if is_stdout {
            eprintln!("- Security Patch: {}", security_patch);
        } else {
            println!("- Security Patch: {}", security_patch);
        }
    }

    // Handle metadata extraction
    #[cfg(feature = "metadata")]
    if let Some(mode) = &args.metadata {
        if !args.list {
            main_pb.set_message("Extracting metadata...");

            let full_mode = mode == "full";

            match save_metadata(&manifest, &args.out, data_offset, full_mode).await {
                Ok(json) => {
                    if is_stdout {
                        println!("{}", json);
                    } else {
                        let mode_str = if full_mode { " (full mode)" } else { "" };
                        println!(
                            "✓ Metadata{} saved to: {}/payload_metadata.json",
                            mode_str,
                            args.out.display()
                        );
                    }
                    multi_progress.clear()?;
                    return Ok(());
                }
                Err(e) => {
                    main_pb.finish_with_message("Failed to save metadata");
                    return Err(e);
                }
            }
        }
    }

    // Handle list command
    if args.list {
        main_pb.finish_and_clear();
        multi_progress.clear()?;

        #[cfg(feature = "metadata")]
        if let Some(mode) = &args.metadata {
            let full_mode = mode == "full";

            match save_metadata(&manifest, &args.out, data_offset, full_mode).await {
                Ok(json) => {
                    if is_stdout {
                        println!("{}", json);
                        return Ok(());
                    } else {
                        let mode_str = if full_mode { " (full mode)" } else { "" };
                        println!(
                            "✓ Metadata{} saved to: {}/payload_metadata.json",
                            mode_str,
                            args.out.display()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Failed to save metadata: {}", e);
                }
            }
        }

        println!();
        return list_partitions(&args.payload_path).await;
    }

    let block_size = manifest.block_size.unwrap_or(4096);

    // Determine partitions to extract
    let partitions_to_extract: Vec<PartitionUpdate> = if args.images.is_empty() {
    manifest.partitions.clone()
} else {
    let images = args.images.split(',').collect::<HashSet<_>>();
    manifest
        .partitions
        .iter()
        .filter(|p| images.contains(p.partition_name.as_str()))
        .cloned()  // Clone each partition
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

    let use_parallel = !args.no_parallel;

    main_pb.set_message(if use_parallel {
        "Extracting Partitions (async parallel)..."
    } else {
        "Processing partitions (async sequential)..."
    });

    // Create shared async payload reader
    let payload_reader = Arc::new(AsyncPayloadReader::new(args.payload_path.clone()).await?);

    let multi_progress = Arc::new(multi_progress);
    let args = Arc::new(args);

    let mut failed_partitions = Vec::new();

    if use_parallel {
        // Parallel async extraction
        let mut tasks = Vec::new();

        for partition in &partitions_to_extract {
            let partition = partition.clone();
            let payload_reader = Arc::clone(&payload_reader);
            let args = Arc::clone(&args);
            let multi_progress = Arc::clone(&multi_progress);

            let task = tokio::spawn(async move {
                let partition_name = partition.partition_name.clone();
                
                match dump_partition(
                    &partition,
                    data_offset,
                    block_size as u64,
                    &args,
                    &payload_reader,
                    Some(&multi_progress),
                )
                .await
                {
                    Ok(()) => Ok(()),
                    Err(e) => Err((partition_name, e)),
                }
            });

            tasks.push(task);
        }

        // Wait for all tasks to complete
        let results = futures::future::join_all(tasks).await;

        for result in results {
            match result {
                Ok(Ok(())) => {}
                Ok(Err((partition_name, error))) => {
                    eprintln!("Failed to process partition {}: {}", partition_name, error);
                    failed_partitions.push(partition_name);
                }
                Err(e) => {
                    eprintln!("Task panicked: {}", e);
                }
            }
        }
    } else {
        // Sequential async extraction
        for partition in &partitions_to_extract {
            if let Err(e) = dump_partition(
                partition,
                data_offset,
                block_size as u64,
                &args,
                &payload_reader,
                Some(&multi_progress),
            )
            .await
            {
                eprintln!(
                    "Failed to process partition {}: {}",
                    partition.partition_name, e
                );
                failed_partitions.push(partition.partition_name.clone());
            }
        }
    }

    // Verify partitions
    if !args.no_verify {
        main_pb.set_message("Verifying partition hashes...");

        let partitions_to_verify: Vec<&PartitionUpdate> = partitions_to_extract
            .iter()
            .filter(|p| !failed_partitions.contains(&p.partition_name))
            .collect();

        match verify_partitions_hash(&partitions_to_verify, &args, &multi_progress).await {
            Ok(failed_verifications) => {
                if !failed_verifications.is_empty() {
                    eprintln!(
                        "Hash verification failed for {} partitions.",
                        failed_verifications.len()
                    );
                }
            }
            Err(e) => {
                eprintln!("Error during hash verification: {}", e);
            }
        }
    } else {
        main_pb.set_message("Hash verification skipped (--no-verify flag)");
    }

    let elapsed_time = format_elapsed_time(start_time.elapsed());

    if failed_partitions.is_empty() {
        main_pb.finish_with_message(format!(
            "All partitions extracted successfully! (in {})",
            elapsed_time
        ));
        println!(
            "\nExtraction completed successfully in {}. Output directory: {:?}",
            elapsed_time, args.out
        );
    } else {
        main_pb.finish_with_message(format!(
            "Completed with {} failed partitions. (in {})",
            failed_partitions.len(),
            elapsed_time
        ));
        println!(
            "\nExtraction completed with {} failed partitions in {}. Output directory: {:?}",
            failed_partitions.len(),
            elapsed_time,
            args.out
        );
    }

    Ok(())
}