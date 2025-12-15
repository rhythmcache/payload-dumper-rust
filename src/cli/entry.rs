// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use anyhow::Result;
use clap::Parser;
use std::time::Instant;
use tokio::fs;

use crate::cli::args::args_def::Args;
use crate::cli::commands::list::list_partitions;
#[cfg(feature = "metadata")]
use crate::cli::commands::metadata_saver::handle_metadata_extraction;
use crate::cli::payload::extractor::extract_partitions;
use crate::cli::payload::file_detector::{PayloadType, detect_payload_type};
use crate::cli::payload::partition_filter::filter_partitions;
use crate::cli::payload::payload_loader::load_payload;
#[cfg(feature = "prefetch")]
use crate::cli::payload::prefetch_extractor::extract_partitions_prefetch;
use crate::cli::ui::ui_print::UiOutput;
use crate::cli::verification::validator::verify_extracted_partitions;
use payload_dumper::utils::{format_elapsed_time, format_size};

#[tokio::main]
pub async fn run() -> Result<()> {
    let args = Args::parse();

    let is_stdout = args.out.to_string_lossy() == "-";
    let ui = UiOutput::new(args.quiet, is_stdout);

    let thread_count = if args.no_parallel {
        1
    } else {
        args.threads
            .unwrap_or_else(|| (num_cpus::get() * 2).min(32))
    };

    ui.println(format!("- Initialized {} thread(s)", thread_count));

    let start_time = Instant::now();
    let main_pb = ui.create_spinner("Starting...");

    // Display file size if available
    if let Ok(metadata) = fs::metadata(&args.payload_path).await
        && metadata.len() > 1024 * 1024
    {
        ui.pb_eprintln(format!(
            "- Processing file: {}, size: {}",
            args.payload_path.display(),
            format_size(metadata.len())
        ));
    }

    // Create output directory
    if !is_stdout {
        fs::create_dir_all(&args.out).await?;
    }

    // Detect file type
    ui.update_spinner(&main_pb, "Detecting file type...");

    let payload_type = detect_payload_type(
        &args.payload_path,
        args.user_agent.as_deref(),
        args.cookies.as_deref(),
    )
    .await?;

    // Load payload
    ui.update_spinner(&main_pb, "Parsing payload...");

    let payload_info = load_payload(
        &args.payload_path,
        payload_type,
        args.user_agent.as_deref(),
        args.cookies.as_deref(),
        &ui,
    )
    .await?;
    let manifest = payload_info.manifest;
    let data_offset = payload_info.data_offset;

    // Print security patch level
    if let Some(security_patch) = &manifest.security_patch_level {
        ui.pb_eprintln(format!("- Security Patch: {}", security_patch));
    }

    // Handle metadata extraction (early exit)
    #[cfg(feature = "metadata")]
    if let Some(mode) = &args.metadata
        && !args.list
    {
        ui.println("- Extracting metadata...");
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
                ui.clear()?;
                return Ok(());
            }
            Err(e) => {
                ui.finish_spinner(main_pb, "Failed to save metadata");
                return Err(e);
            }
        }
    }

    // Handle list command
    if args.list {
        ui.clear()?;

        // Save metadata if requested in list mode
        #[cfg(feature = "metadata")]
        if let Some(mode) = &args.metadata {
            if let Err(e) = handle_metadata_extraction(
                &manifest,
                &args.out,
                data_offset,
                mode,
                &args.images,
                is_stdout,
            )
            .await
            {
                ui.error(format!("Failed to save metadata: {}", e));
            }
            if is_stdout {
                return Ok(());
            }
        }

        println!();
        list_partitions(&manifest);
        return Ok(());
    }

    let block_size = manifest.block_size.unwrap_or(4096);

    // Filter partitions to extract
    let partitions_to_extract = filter_partitions(&manifest, &args.images);

    if partitions_to_extract.is_empty() {
        ui.finish_spinner(main_pb, "No partitions to extract");
        ui.clear()?;
        return Ok(());
    }

    ui.println(format!(
        "- Found {} partitions to extract",
        partitions_to_extract.len()
    ));

    // Check for prefetch mode (remote URLs only)
    let is_remote = matches!(
        payload_type,
        PayloadType::RemoteZip | PayloadType::RemoteBin
    );

    let failed_partitions = if args.prefetch && is_remote {
        #[cfg(feature = "prefetch")]
        {
            ui.println("- Using prefetch mode for remote extraction");
            ui.update_spinner(&main_pb, "Downloading and extracting partitions...");

            let url = args.payload_path.to_string_lossy().to_string();

            // Get payload offset (0 for .bin, non-zero for ZIP)
            let payload_offset = match payload_type {
                PayloadType::RemoteZip => {
                    // For ZIP files, we need to get the offset of payload.bin inside the ZIP
                    use payload_dumper::zip::core_parser::ZipParser;
                    let http_reader = payload_dumper::http::HttpReader::new(
                        url.clone(),
                        args.user_agent.as_deref(),
                        args.cookies.as_deref(),
                    )
                    .await?;
                    let entry = ZipParser::find_payload_entry(&http_reader).await?;
                    ZipParser::get_data_offset(&http_reader, &entry).await?
                }
                PayloadType::RemoteBin => 0, // Direct .bin file has no offset
                _ => unreachable!(),
            };

            extract_partitions_prefetch(
                &args,
                &partitions_to_extract,
                data_offset,
                block_size as u64,
                url,
                payload_offset,
                &ui,
            )
            .await?
        }
        #[cfg(not(feature = "prefetch"))]
        {
            return Err(anyhow::anyhow!(
                "Prefetch mode requires the 'prefetch' feature"
            ));
        }
    } else {
        // Normal extraction
        ui.update_spinner(
            &main_pb,
            if args.no_parallel {
                "Processing partitions..."
            } else {
                "Extracting partitions..."
            },
        );

        extract_partitions(
            &args,
            &partitions_to_extract,
            data_offset,
            block_size as u64,
            payload_info.reader,
            &ui,
        )
        .await?
    };

    // Verify partitions
    verify_extracted_partitions(&partitions_to_extract, &failed_partitions, &args, &ui).await?;

    // Print completion summary
    let elapsed_time = format_elapsed_time(start_time.elapsed());

    if failed_partitions.is_empty() {
        ui.finish_spinner(
            main_pb,
            format!(
                "All partitions extracted successfully! (in {})",
                elapsed_time
            ),
        );
        ui.println_final(format!(
            "\n- Extraction completed successfully in {}. Output directory: {:?}",
            elapsed_time, args.out,
        ));
    } else {
        ui.finish_spinner(
            main_pb,
            format!(
                "Completed with {} failed partitions. (in {})",
                failed_partitions.len(),
                elapsed_time
            ),
        );
        ui.eprintln_final(format!(
            "\n- Extraction completed with {} failed partitions in {}. Output directory: {:?}",
            failed_partitions.len(),
            elapsed_time,
            args.out,
        ));
    }

    Ok(())
}
