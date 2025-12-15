// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use crate::cli::args::args_def::Args;
use crate::cli::ui::cli_reporter::{CliDownloadReporter, CliExtractionReporter};
use crate::cli::ui::ui_print::UiOutput;
use anyhow::Result;
use payload_dumper::http::HttpReader;
use payload_dumper::prefetch::{
    ExtractionPaths, PartitionExtractionConfig, prefetch_and_dump_partition,
};
use payload_dumper::structs::PartitionUpdate;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Semaphore;

/// extract partitions using prefetch mode (download then extract)
pub async fn extract_partitions_prefetch(
    args: &Args,
    partitions: &[PartitionUpdate],
    data_offset: u64,
    block_size: u64,
    url: String,
    payload_offset: u64,
    ui: &UiOutput,
) -> Result<Vec<String>> {
    let config = PartitionExtractionConfig {
        data_offset,
        block_size,
        payload_offset,
    };

    if args.no_parallel {
        extract_prefetch_sequential(args, partitions, &config, url, ui).await
    } else {
        let thread_count = args.threads.unwrap_or_else(num_cpus::get);

        extract_prefetch_parallel(args, partitions, &config, url, thread_count, ui).await
    }
}

/// sequential prefetch extraction
async fn extract_prefetch_sequential(
    args: &Args,
    partitions: &[PartitionUpdate],
    config: &PartitionExtractionConfig,
    url: String,
    ui: &UiOutput,
) -> Result<Vec<String>> {
    let mut failed_partitions = Vec::new();
    let temp_dir = TempDir::new()?;

    let http_reader =
        HttpReader::new(url, args.user_agent.as_deref(), args.cookies.as_deref()).await?;

    for partition in partitions {
        let partition_name = &partition.partition_name;
        let paths = ExtractionPaths {
            temp_path: temp_dir.path().join(format!("{}.prefetch", partition_name)),
            output_path: args.out.join(format!("{}.img", partition_name)),
        };
        let download_progress = ui.create_download_progress("");
        let extraction_progress = ui.create_extraction_progress(partition_name);

        let download_reporter = CliDownloadReporter::new(download_progress);
        let extraction_reporter = CliExtractionReporter::new(extraction_progress);

        if let Err(e) = prefetch_and_dump_partition(
            partition,
            config,
            &http_reader,
            paths,
            &download_reporter,
            &extraction_reporter,
        )
        .await
        {
            ui.error(format!(
                "Failed to prefetch/extract partition {}: {}",
                partition_name, e
            ));
            failed_partitions.push(partition_name.clone());
        }
    }

    Ok(failed_partitions)
}

/// parallel prefetch extraction with thread limiting
async fn extract_prefetch_parallel(
    args: &Args,
    partitions: &[PartitionUpdate],
    config: &PartitionExtractionConfig,
    url: String,
    thread_count: usize,
    ui: &UiOutput,
) -> Result<Vec<String>> {
    let temp_dir = TempDir::new()?;
    let temp_dir_path = temp_dir.path().to_path_buf();

    let http_reader =
        Arc::new(HttpReader::new(url, args.user_agent.as_deref(), args.cookies.as_deref()).await?);

    let semaphore = Arc::new(Semaphore::new(thread_count));
    let mut tasks = Vec::new();
    let out_dir = args.out.clone();
    let config = config.clone();

    for partition in partitions {
        let partition = partition.clone();
        let partition_name = partition.partition_name.clone();
        let http_reader = Arc::clone(&http_reader);
        let semaphore = Arc::clone(&semaphore);
        let temp_dir_path = temp_dir_path.clone();
        let out_dir = out_dir.clone();
        let config = config.clone();
        let download_progress = ui.create_download_progress("");
        let extraction_progress = ui.create_extraction_progress(&partition_name);

        let task = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();

            let paths = ExtractionPaths {
                temp_path: temp_dir_path.join(format!("{}.prefetch", partition_name)),
                output_path: out_dir.join(format!("{}.img", partition_name)),
            };

            let download_reporter = CliDownloadReporter::new(download_progress);
            let extraction_reporter = CliExtractionReporter::new(extraction_progress);

            match prefetch_and_dump_partition(
                &partition,
                &config,
                &http_reader,
                paths,
                &download_reporter,
                &extraction_reporter,
            )
            .await
            {
                Ok(()) => Ok(()),
                Err(e) => Err((partition_name, e)),
            }
        });

        tasks.push(task);
    }

    // wait for all tasks
    let results = futures::future::join_all(tasks).await;
    let mut failed_partitions = Vec::new();

    for result in results {
        match result {
            Ok(Ok(())) => {}
            Ok(Err((partition_name, error))) => {
                ui.error(format!(
                    "Failed to prefetch/extract partition {}: {}",
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
