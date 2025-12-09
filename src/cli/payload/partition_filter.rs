// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use ahash::AHashSet as HashSet;
use payload_dumper::DeltaArchiveManifest;
use payload_dumper::PartitionUpdate;

/// filters partitions based on the images argument
/// returns all partitions if images is empty, otherwise returns filtered list
pub fn filter_partitions(
    manifest: &DeltaArchiveManifest,
    images_arg: &str,
) -> Vec<PartitionUpdate> {
    if images_arg.is_empty() {
        manifest.partitions.clone()
    } else {
        let images: HashSet<&str> = images_arg.split(',').collect();
        manifest
            .partitions
            .iter()
            .filter(|p| images.contains(p.partition_name.as_str()))
            .cloned()
            .collect()
    }
}
