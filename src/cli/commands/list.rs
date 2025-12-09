// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use payload_dumper::DeltaArchiveManifest;
use payload_dumper::utils::format_size;

pub fn list_partitions(manifest: &DeltaArchiveManifest) {
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
}
