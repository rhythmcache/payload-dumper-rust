// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust
//
// This file is part of payload-dumper-rust. It implements components used for
// extracting and processing Android OTA payloads.

use crate::DeltaArchiveManifest;
use crate::cow_merge_operation;
use crate::install_operation;
use crate::structs::*;
use crate::utils::format_size;
use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use anyhow::Result;
use std::path::Path;
use tokio::fs;

pub async fn handle_metadata_extraction(
    manifest: &DeltaArchiveManifest,
    out_dir: &std::path::Path,
    data_offset: u64,
    mode: &str,
    images_filter: &str,
    is_stdout: bool,
) -> Result<()> {
    let full_mode = mode == "full";
    let filter_partitions = if !images_filter.is_empty() {
        let images: HashSet<&str> = images_filter.split(',').collect();
        Some(images)
    } else {
        None
    };

    match save_metadata(
        manifest,
        out_dir,
        data_offset,
        full_mode,
        filter_partitions.as_ref(),
    )
    .await
    {
        Ok(json) => {
            if is_stdout {
                println!("{}", json);
            } else {
                let mode_str = if full_mode { " (full mode)" } else { "" };
                let filter_str = if filter_partitions.is_some() {
                    format!(" for {} partition(s)", images_filter.split(',').count())
                } else {
                    String::new()
                };
                println!(
                    "✓ Metadata{}{} saved to: {}/payload_metadata.json",
                    mode_str,
                    filter_str,
                    out_dir.display()
                );
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn save_metadata(
    manifest: &DeltaArchiveManifest,
    output_dir: &Path,
    data_offset: u64,
    full_mode: bool,
    filter_partitions: Option<&HashSet<&str>>,
) -> Result<String> {
    let mut partitions = Vec::new();
    let mut total_payload_size = 0u64;
    let mut total_operations = 0usize;
    let mut global_op_stats: HashMap<String, (usize, u64)> = HashMap::new();

    for partition in &manifest.partitions {
        // Skip partition if filter is provided and partition is not in filter
        if let Some(filter) = filter_partitions
            && !filter.contains(partition.partition_name.as_str()) {
                continue;
            }

        if let Some(info) = &partition.new_partition_info {
            let size_in_bytes = info.size.unwrap_or(0);
            let block_size = manifest.block_size.unwrap_or(4096) as u64;
            let size_in_blocks = size_in_bytes / block_size;
            let total_blocks = size_in_bytes / block_size;
            let hash = info.hash.as_ref().map(hex::encode);

            let mut start_offset = data_offset;
            for op in &partition.operations {
                if let Some(_first_extent) = op.dst_extents.first() {
                    start_offset = data_offset + op.data_offset.unwrap_or(0);
                    break;
                }
            }
            let end_offset = start_offset + size_in_bytes;

            // Extract complete operation details
            let mut operations_list = Vec::new();
            let mut op_type_stats: HashMap<String, (usize, u64)> = HashMap::new();
            let mut total_data_size = 0u64;
            let mut num_src_extents = 0usize;
            let mut num_dst_extents = 0usize;

            for (idx, op) in partition.operations.iter().enumerate() {
                let op_type_name = op.r#type().as_str_name().to_string();
                let data_len = op.data_length.unwrap_or(0);

                // Only extract full operation details in full mode
                if full_mode {
                    let src_extents: Vec<ExtentInfo> = op
                        .src_extents
                        .iter()
                        .map(|ext| ExtentInfo {
                            start_block: ext.start_block.unwrap_or(0),
                            num_blocks: ext.num_blocks.unwrap_or(0),
                        })
                        .collect();

                    let dst_extents: Vec<ExtentInfo> = op
                        .dst_extents
                        .iter()
                        .map(|ext| ExtentInfo {
                            start_block: ext.start_block.unwrap_or(0),
                            num_blocks: ext.num_blocks.unwrap_or(0),
                        })
                        .collect();

                    operations_list.push(InstallOperationInfo {
                        operation_type: op_type_name.clone(),
                        operation_index: idx,
                        data_offset: op.data_offset,
                        data_length: op.data_length,
                        data_length_readable: op.data_length.map(format_size),
                        src_extents: src_extents.clone(),
                        src_length: op.src_length,
                        dst_extents: dst_extents.clone(),
                        dst_length: op.dst_length,
                        data_sha256_hash: op.data_sha256_hash.as_ref().map(hex::encode),
                        src_sha256_hash: op.src_sha256_hash.as_ref().map(hex::encode),
                    });

                    num_src_extents += op.src_extents.len();
                    num_dst_extents += op.dst_extents.len();
                } else {
                    num_src_extents += op.src_extents.len();
                    num_dst_extents += op.dst_extents.len();
                }

                let entry = op_type_stats.entry(op_type_name.clone()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += data_len;

                let global_entry = global_op_stats.entry(op_type_name).or_insert((0, 0));
                global_entry.0 += 1;
                global_entry.1 += data_len;

                total_data_size += data_len;
            }

            total_payload_size += total_data_size;
            total_operations += partition.operations.len();

            let operation_type_stats: Vec<OperationTypeStats> = op_type_stats
                .into_iter()
                .map(|(op_type, (count, size))| OperationTypeStats {
                    operation_type: op_type,
                    count,
                    total_data_size: size,
                })
                .collect();

            let compression_type = partition
                .operations
                .iter()
                .find_map(|op| match op.r#type() {
                    install_operation::Type::ReplaceXz => Some("xz"),
                    install_operation::Type::ReplaceBz => Some("bz2"),
                    install_operation::Type::Zstd => Some("zstd"),
                    _ => None,
                })
                .unwrap_or("none")
                .to_string();

            let encryption = if partition.partition_name.contains("userdata") {
                "AES"
            } else {
                "none"
            };

            let old_partition_info =
                partition
                    .old_partition_info
                    .as_ref()
                    .map(|old_info| PartitionInfoDetails {
                        size: old_info.size.unwrap_or(0),
                        hash: old_info.hash.as_ref().map(hex::encode),
                    });

            let hash_tree_data_extent =
                partition
                    .hash_tree_data_extent
                    .as_ref()
                    .map(|ext| ExtentInfo {
                        start_block: ext.start_block.unwrap_or(0),
                        num_blocks: ext.num_blocks.unwrap_or(0),
                    });

            let hash_tree_extent = partition.hash_tree_extent.as_ref().map(|ext| ExtentInfo {
                start_block: ext.start_block.unwrap_or(0),
                num_blocks: ext.num_blocks.unwrap_or(0),
            });

            let fec_data_extent = partition.fec_data_extent.as_ref().map(|ext| ExtentInfo {
                start_block: ext.start_block.unwrap_or(0),
                num_blocks: ext.num_blocks.unwrap_or(0),
            });

            let fec_extent = partition.fec_extent.as_ref().map(|ext| ExtentInfo {
                start_block: ext.start_block.unwrap_or(0),
                num_blocks: ext.num_blocks.unwrap_or(0),
            });

            let merge_operations: Vec<MergeOperationInfo> = partition
                .merge_operations
                .iter()
                .map(|merge_op| {
                    let op_type = if let Some(t) = merge_op.r#type {
                        cow_merge_operation::Type::try_from(t)
                            .map(|t| t.as_str_name().to_string())
                            .unwrap_or_else(|_| "UNKNOWN".to_string())
                    } else {
                        "UNKNOWN".to_string()
                    };

                    MergeOperationInfo {
                        operation_type: op_type,
                        src_extent: merge_op.src_extent.as_ref().map(|ext| ExtentInfo {
                            start_block: ext.start_block.unwrap_or(0),
                            num_blocks: ext.num_blocks.unwrap_or(0),
                        }),
                        dst_extent: merge_op.dst_extent.as_ref().map(|ext| ExtentInfo {
                            start_block: ext.start_block.unwrap_or(0),
                            num_blocks: ext.num_blocks.unwrap_or(0),
                        }),
                        src_offset: merge_op.src_offset,
                    }
                })
                .collect();

            let new_partition_signatures: Vec<SignatureInfo> = partition
                .new_partition_signature
                .iter()
                .map(|sig| SignatureInfo {
                    data: sig.data.as_ref().map(hex::encode),
                    unpadded_signature_size: sig.unpadded_signature_size,
                })
                .collect();

            let estimate_cow_size_readable = partition.estimate_cow_size.map(format_size);

            partitions.push(PartitionMetadata {
                partition_name: partition.partition_name.clone(),
                size_in_blocks,
                size_in_bytes,
                size_readable: format_size(size_in_bytes),
                hash,
                start_offset,
                end_offset,
                data_offset,
                partition_type: partition.partition_name.clone(),
                operations_count: partition.operations.len(),
                compression_type,
                encryption: encryption.to_string(),
                block_size,
                total_blocks,
                run_postinstall: partition.run_postinstall,
                postinstall_path: partition.postinstall_path.clone(),
                filesystem_type: partition.filesystem_type.clone(),
                postinstall_optional: partition.postinstall_optional,
                hash_tree_algorithm: partition.hash_tree_algorithm.clone(),
                version: partition.version.clone(),
                old_partition_info,
                hash_tree_salt: partition.hash_tree_salt.as_ref().map(hex::encode),
                hash_tree_data_extent,
                hash_tree_extent,
                fec_data_extent,
                fec_extent,
                fec_roots: partition.fec_roots,
                estimate_cow_size: partition.estimate_cow_size,
                estimate_cow_size_readable,
                estimate_op_count_max: partition.estimate_op_count_max,
                operations: operations_list,
                merge_operations,
                merge_operations_count: partition.merge_operations.len(),
                new_partition_signatures,
                signature_count: partition.new_partition_signature.len(),
                operation_type_stats,
                total_data_size,
                total_data_size_readable: format_size(total_data_size),
                num_src_extents,
                num_dst_extents,
            });
        }
    }

    let dynamic_partition_metadata = if let Some(dpm) = &manifest.dynamic_partition_metadata {
        let groups: Vec<DynamicPartitionGroupInfo> = dpm
            .groups
            .iter()
            .map(|group| DynamicPartitionGroupInfo {
                name: group.name.clone(),
                size: group.size,
                size_readable: group.size.map(format_size),
                partition_names: group.partition_names.clone(),
                partition_count: group.partition_names.len(),
            })
            .collect();

        let vabc_feature_set = dpm.vabc_feature_set.as_ref().map(|fs| VabcFeatureSetInfo {
            threaded: fs.threaded,
            batch_writes: fs.batch_writes,
        });

        Some(DynamicPartitionInfo {
            groups_count: dpm.groups.len(),
            groups,
            snapshot_enabled: dpm.snapshot_enabled,
            vabc_enabled: dpm.vabc_enabled,
            vabc_compression_param: dpm.vabc_compression_param.clone(),
            cow_version: dpm.cow_version,
            vabc_feature_set,
            compression_factor: dpm.compression_factor,
        })
    } else {
        None
    };

    let apex_info: Vec<ApexInfoMetadata> = manifest
        .apex_info
        .iter()
        .map(|info| ApexInfoMetadata {
            package_name: info.package_name.clone(),
            version: info.version,
            is_compressed: info.is_compressed,
            decompressed_size: info.decompressed_size,
            decompressed_size_readable: info.decompressed_size.map(|s| format_size(s as u64)),
        })
        .collect();

    let global_operation_stats: Vec<OperationTypeStats> = global_op_stats
        .into_iter()
        .map(|(op_type, (count, size))| OperationTypeStats {
            operation_type: op_type,
            count,
            total_data_size: size,
        })
        .collect();

    let payload_metadata = PayloadMetadata {
        security_patch_level: manifest.security_patch_level.clone(),
        block_size: manifest.block_size.unwrap_or(4096),
        minor_version: manifest.minor_version.unwrap_or(0),
        max_timestamp: manifest.max_timestamp,
        dynamic_partition_metadata,
        partial_update: manifest.partial_update,
        apex_info_count: manifest.apex_info.len(),
        apex_info,
        partitions_count: partitions.len(),
        partitions,
        signatures_offset: manifest.signatures_offset,
        signatures_size: manifest.signatures_size,
        total_payload_size,
        total_payload_size_readable: format_size(total_payload_size),
        total_operations_count: total_operations,
        global_operation_stats,
    };

    let json = serde_json::to_string_pretty(&payload_metadata)?;

    if output_dir.to_string_lossy() == "-" {
        return Ok(json);
    }

    let metadata_path = output_dir.join("payload_metadata.json");
    fs::write(metadata_path, &json).await?;

    Ok(json)
}
