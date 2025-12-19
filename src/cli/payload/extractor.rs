// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use crate::cli::args::args_def::Args;
use crate::cli::ui::cli_reporter::CliExtractionReporter;
use crate::cli::ui::ui_print::UiOutput;
use anyhow::Result;
use payload_dumper::payload::payload_dumper::{AsyncPayloadRead, dump_partition};
use payload_dumper::structs::PartitionUpdate;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// extracts partitions using parallel or sequential processing
/// returns a list of failed partition names
pub async fn extract_partitions(
    args: &Args,
    partitions: &[PartitionUpdate],
    data_offset: u64,
    block_size: u64,
    payload_reader: Arc<dyn AsyncPayloadRead>,
    ui: &UiOutput,
) -> Result<Vec<String>> {
    if args.no_parallel {
        extract_sequential(
            args,
            partitions,
            data_offset,
            block_size,
            payload_reader,
            ui,
        )
        .await
    } else {
        let thread_count = args
            .threads
            .unwrap_or_else(|| (num_cpus::get() * 2).min(32));
        extract_parallel(
            args,
            partitions,
            data_offset,
            block_size,
            payload_reader,
            thread_count,
            ui,
        )
        .await
    }
}

/// sequential extraction
async fn extract_sequential(
    args: &Args,
    partitions: &[PartitionUpdate],
    data_offset: u64,
    block_size: u64,
    payload_reader: Arc<dyn AsyncPayloadRead>,
    ui: &UiOutput,
) -> Result<Vec<String>> {
    let mut failed_partitions = Vec::new();

    for partition in partitions {
        // Create progress through UI layer - no indicatif imports needed!
        let progress = ui.create_extraction_progress(&partition.partition_name);
        let reporter = CliExtractionReporter::new(progress);
        let output_path = args.out.join(format!("{}.img", &partition.partition_name));

        if let Err(e) = dump_partition(
            partition,
            data_offset,
            block_size,
            output_path,
            &payload_reader,
            &reporter,
            Some(args.source_dir.clone()),
        )
        .await
        {
            ui.error(format!(
                "Failed to process partition {}: {}",
                partition.partition_name, e
            ));
            failed_partitions.push(partition.partition_name.clone());
        }
    }

    Ok(failed_partitions)
}

/// parallel extraction with thread limiting
async fn extract_parallel(
    args: &Args,
    partitions: &[PartitionUpdate],
    data_offset: u64,
    block_size: u64,
    payload_reader: Arc<dyn AsyncPayloadRead>,
    thread_count: usize,
    ui: &UiOutput,
) -> Result<Vec<String>> {
    let semaphore = Arc::new(Semaphore::new(thread_count));
    let mut tasks = Vec::new();
    let out_dir = args.out.clone();
    let source_dir = args.source_dir.clone();

    for partition in partitions {
        let partition = partition.clone();
        let payload_reader = Arc::clone(&payload_reader);
        let out_dir = out_dir.clone();
        let source_dir = source_dir.clone();
        let semaphore = Arc::clone(&semaphore);
        let progress = ui.create_extraction_progress(&partition.partition_name);

        let task = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();

            let partition_name = partition.partition_name.clone();
            let output_path = out_dir.join(format!("{}.img", partition_name));
            let reporter = CliExtractionReporter::new(progress);

            match dump_partition(
                &partition,
                data_offset,
                block_size,
                output_path,
                &payload_reader,
                &reporter,
                Some(source_dir),
            )
            .await
            {
                Ok(()) => Ok(()),
                Err(e) => Err((partition_name, e)),
            }
        });

        tasks.push(task);
    }

    // wait for all tasks to complete
    let results = futures::future::join_all(tasks).await;
    let mut failed_partitions = Vec::new();

    for result in results {
        match result {
            Ok(Ok(())) => {}
            Ok(Err((partition_name, error))) => {
                ui.error(format!(
                    "Failed to process partition {}: {}",
                    partition_name, error
                ));
                failed_partitions.push(partition_name);
            }
            Err(e) => {
                ui.error(format!("Task panicked: {}", e));
            }
        }
    }

    Ok(failed_partitions)
}
