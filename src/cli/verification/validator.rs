// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use crate::cli::args::args_def::Args;
use crate::cli::ui::ui_print::UiOutput;
use crate::cli::verification::verify::verify_partitions_hash;
use anyhow::Result;
use payload_dumper::structs::PartitionUpdate;

/// verifies partition hashes for successfully extracted partitions
/// returns a list of partition names that failed verification
pub async fn verify_extracted_partitions(
    partitions: &[PartitionUpdate],
    failed_extractions: &[String],
    args: &Args,
    ui: &UiOutput,
) -> Result<Vec<String>> {
    if args.no_verify {
        ui.println("- Skipping hash verification");
        return Ok(Vec::new());
    }

    ui.println("- Verifying partition hashes...");

    let partitions_to_verify: Vec<&PartitionUpdate> = partitions
        .iter()
        .filter(|p| !failed_extractions.contains(&p.partition_name))
        .collect();

    match verify_partitions_hash(&partitions_to_verify, args, ui).await {
        Ok(failed_verifications) => {
            if !failed_verifications.is_empty() {
                ui.error(format!(
                    "Hash verification failed for {} partitions.",
                    failed_verifications.len()
                ));
            }
            Ok(failed_verifications)
        }
        Err(e) => {
            ui.error(format!("Error during hash verification: {}", e));
            Err(e)
        }
    }
}
