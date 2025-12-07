use ahash::AHashSet as HashSet;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow};

use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::fs;
use tokio::sync::Semaphore;

mod args;
mod constants;
#[cfg(feature = "remote_zip")]
mod http;
#[cfg(feature = "metadata")]
mod metadata;
mod payload;
#[cfg(feature = "prefetch")]
mod prefetch;
mod readers;
#[cfg(feature = "metadata")]
mod structs;
mod utils;
mod verify;
#[cfg(any(feature = "local_zip", feature = "remote_zip"))]
mod zip;

use crate::args::Args;
#[cfg(feature = "metadata")]
use crate::metadata::handle_metadata_extraction;
use crate::payload::payload_dumper::{AsyncPayloadRead, dump_partition};
use crate::payload::payload_parser::parse_local_payload;
#[cfg(feature = "local_zip")]
use crate::payload::payload_parser::parse_local_zip_payload;
#[cfg(feature = "remote_zip")]
use crate::payload::payload_parser::parse_remote_payload;
use crate::readers::local_reader::LocalAsyncPayloadReader;
#[cfg(feature = "local_zip")]
use crate::readers::local_zip_reader::LocalAsyncZipPayloadReader;
#[cfg(feature = "remote_zip")]
use crate::readers::remote_zip_reader::RemoteAsyncZipPayloadReader;
use crate::utils::FileType;
#[cfg(feature = "remote_zip")]
use crate::utils::detect_remote_file_type;
use crate::utils::{detect_file_type, format_elapsed_time, format_size, list_partitions};
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
        (num_cpus::get() * 2).min(32) // cap at 32 threads
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
    main_pb.enable_steady_tick(tokio::time::Duration::from_millis(300));

    let payload_path_str = args.payload_path.to_string_lossy().to_string();

    let is_url =
        payload_path_str.starts_with("http://") || payload_path_str.starts_with("https://");

    // check if we're outputting to stdout
    let is_stdout = args.out.to_string_lossy() == "-";

    // detect file type
    let extension = args
        .payload_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let mut is_zip = extension == "zip";
    let mut is_bin = extension == "bin" || extension.is_empty();

    // if extension detection is inconclusive, check magic bytes
    if !is_url && (!is_zip && !is_bin || extension.is_empty()) {
        match detect_file_type(&args.payload_path).await {
            Ok(FileType::Zip) => {
                is_zip = true;
                is_bin = false;
            }
            Ok(FileType::Bin) => {
                is_bin = true;
                is_zip = false;
            }
            Err(e) => {
                return Err(anyhow!(
                    "Unable to detect file type. Extension: '{}', Error: {}",
                    extension,
                    e
                ));
            }
        }
    }

    // if URL has no clear extension, check magic bytes remotely
    if is_url && extension.is_empty() {
        #[cfg(feature = "remote_zip")]
        {
            if !is_stdout {
                main_pb.set_message("Detecting remote file type...");
            }

            match detect_remote_file_type(&payload_path_str, args.user_agent.as_deref()).await {
                Ok(FileType::Zip) => {
                    is_zip = true;
                    is_bin = false;
                }

                Ok(FileType::Bin) => {
                    return Err(anyhow!(
                        "Direct .bin URLs are not supported. Only ZIP files containing payload.bin are supported for remote URLs."
                    ));
                }
                Err(e) => {
                    return Err(anyhow!("Unable to detect remote file type: {}", e));
                }
            }
        }
    }

    if !is_zip && !is_bin {
        return Err(anyhow!(
            "Unsupported file type. Only .bin and .zip files are supported"
        ));
    }

    // validate feature requirements
    if is_url {
        #[cfg(not(feature = "remote_zip"))]
        return Err(anyhow!(
            "Remote URL processing requires the 'remote_zip' feature to be enabled. Please recompile with --features remote_zip"
        ));
    }

    if is_zip && !is_url {
        #[cfg(not(feature = "local_zip"))]
        return Err(anyhow!(
            "Local ZIP file processing requires the 'local_zip' feature to be enabled. Please recompile with --features local_zip"
        ));
    }

    main_pb.set_message("Opening file...");

    // Get file metadata
    if let Ok(metadata) = fs::metadata(&args.payload_path).await
        && metadata.len() > 1024 * 1024
    {
        if is_stdout {
            eprintln!(
                "- Processing file: {}, size: {}",
                payload_path_str,
                format_size(metadata.len())
            );
        } else {
            main_pb.println(format!(
                "- Processing file: {}, size: {}",
                payload_path_str,
                format_size(metadata.len())
            ));
        }
    }

    if args.out.to_string_lossy() != "-" {
        fs::create_dir_all(&args.out).await?;
    }

    main_pb.set_message("Parsing payload...");

    let (manifest, data_offset) = if is_url {
        #[cfg(feature = "remote_zip")]
        {
            if !is_zip {
                return Err(anyhow!(
                    "Remote URLs must point to ZIP files containing payload.bin\n\
                 Direct .bin URLs are not supported"
                ));
            }

            if !is_stdout {
                main_pb.println("- Connecting to remote ZIP archive...");
            }
            let (manifest, data_offset, content_length) =
                parse_remote_payload(payload_path_str.clone(), args.user_agent.as_deref()).await?;

            // Print the ZIP size immediately after connecting
            if is_stdout {
                eprintln!("- Remote ZIP size: {}", format_size(content_length));
            } else {
                main_pb.println(format!(
                    "- Remote ZIP size: {}",
                    format_size(content_length)
                ));
            }

            (manifest, data_offset)
        }
        #[cfg(not(feature = "remote_zip"))]
        {
            unreachable!();
        }
    } else if is_zip {
        #[cfg(feature = "local_zip")]
        {
            parse_local_zip_payload(args.payload_path.clone()).await?
        }
        #[cfg(not(feature = "local_zip"))]
        {
            unreachable!();
        }
    } else {
        // Local .bin file
        parse_local_payload(&args.payload_path).await?
    };

    // Print security patch level
    if let Some(security_patch) = &manifest.security_patch_level {
        if is_stdout {
            eprintln!("- Security Patch: {}", security_patch);
        } else {
            main_pb.println(format!("- Security Patch: {}", security_patch));
        }
    }

    #[cfg(feature = "metadata")]
    if let Some(mode) = &args.metadata
        && !args.list
    {
        main_pb.println("- Extracting metadata...");

        match handle_metadata_extraction(
            &manifest,
            &args.out,
            data_offset,
            mode,
            &args.images,
            is_stdout,
        )
        .await
        {
            Ok(()) => {
                multi_progress.clear()?;
                return Ok(());
            }
            Err(e) => {
                main_pb.finish_with_message("Failed to save metadata");
                return Err(e);
            }
        }
    }

    // handle list command
    if args.list {
        main_pb.finish_and_clear();
        multi_progress.clear()?;

        // if metadata was requested in list mode, save it
        #[cfg(feature = "metadata")]
        if let Some(mode) = &args.metadata {
            match handle_metadata_extraction(
                &manifest,
                &args.out,
                data_offset,
                mode,
                &args.images,
                is_stdout,
            )
            .await
            {
                Ok(()) => {
                    if is_stdout {
                        return Ok(());
                    }
                }
                Err(e) => {
                    eprintln!("Failed to save metadata: {}", e);
                }
            }
        }

        println!();
        list_partitions(&manifest);
        return Ok(());
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
            .cloned() // Clone each partition
            .collect()
    };

    if partitions_to_extract.is_empty() {
        main_pb.finish_with_message("No partitions to extract");
        multi_progress.clear()?;
        return Ok(());
    }

    main_pb.println(format!(
        "- Found {} partitions to extract",
        partitions_to_extract.len()
    ));

    // Check if prefetch mode is enabled for remote URLs
    if args.prefetch && is_url {
        #[cfg(feature = "prefetch")]
        {
            if !is_stdout {
                main_pb.println("- Using prefetch mode for remote extraction");
            }

            let multi_progress = Arc::new(multi_progress);
            let args = Arc::new(args);

            return crate::prefetch::prefetch_and_extract(
                payload_path_str.clone(),
                manifest,
                data_offset,
                Arc::clone(&args),
                partitions_to_extract,
                Arc::clone(&multi_progress),
            )
            .await;
        }
        #[cfg(not(feature = "prefetch"))]
        {
            return Err(anyhow!(
                "Prefetch mode requires the 'prefetch' feature to be enabled"
            ));
        }
    }

    // Prefetch not enabled - continue with normal extraction
    let use_parallel = !args.no_parallel;

    main_pb.set_message(if use_parallel {
        "Extracting Partitions..."
    } else {
        "Processing partitions..."
    });

    let payload_reader: Arc<dyn AsyncPayloadRead> = if is_url {
        #[cfg(feature = "remote_zip")]
        {
            // Remote URL
            if !is_stdout {
                main_pb.println("- Preparing remote extraction...");
            }

            let remote_reader = RemoteAsyncZipPayloadReader::new(
                payload_path_str.clone(),
                args.user_agent.as_deref(),
            )
            .await?;

            // Print remote ZIP size (for non-prefetch mode)
            if is_stdout {
                eprintln!(
                    "- Remote ZIP size: {}",
                    format_size(remote_reader.http_reader.content_length)
                );
            } else {
                main_pb.println(format!(
                    "- Remote ZIP size: {}",
                    format_size(remote_reader.http_reader.content_length)
                ));
            }

            Arc::new(remote_reader)
        }
        #[cfg(not(feature = "remote_zip"))]
        {
            unreachable!(); // This should be caught by the validation above
        }
    } else if is_zip {
        #[cfg(feature = "local_zip")]
        {
            Arc::new(LocalAsyncZipPayloadReader::new(args.payload_path.clone()).await?)
        }
        #[cfg(not(feature = "local_zip"))]
        {
            unreachable!(); // This should be caught by the validation above
        }
    } else {
        Arc::new(LocalAsyncPayloadReader::new(args.payload_path.clone()).await?)
    };

    let multi_progress = Arc::new(multi_progress);
    let args = Arc::new(args);

    let mut failed_partitions = Vec::new();

    if use_parallel {
        // thread limiting
        let semaphore = Arc::new(Semaphore::new(thread_count));
        let mut tasks = Vec::new();

        for partition in &partitions_to_extract {
            let partition = partition.clone();
            let payload_reader = Arc::clone(&payload_reader);
            let args = Arc::clone(&args);
            let multi_progress = Arc::clone(&multi_progress);
            let semaphore = Arc::clone(&semaphore);

            let task = tokio::spawn(async move {
                // acquire permit to limit concurrent tasks to thread_count
                let _permit = semaphore.acquire().await.unwrap();

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
                // permit is automatically released when _permit is dropped
            });

            tasks.push(task);
        }

        // wait for all tasks to complete
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
        main_pb.println("- Verifying partition hashes...");

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
        main_pb.println("- Skipping hash verification");
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
