use crate::DeltaArchiveManifest;
use crate::install_operation;
use crate::module::structs::{
    ApexInfoMetadata, DynamicPartitionGroupInfo, DynamicPartitionInfo, PartitionMetadata,
    PayloadMetadata, VabcFeatureSetInfo,
};
use crate::module::utils::format_size;
use anyhow::Result;
use serde_json;
use std::fs;
use std::path::Path;

pub fn save_metadata(
    manifest: &DeltaArchiveManifest,
    output_dir: &Path,
    data_offset: u64,
) -> Result<String> {
    let mut partitions = Vec::new();
    for partition in &manifest.partitions {
        if let Some(info) = &partition.new_partition_info {
            let size_in_bytes = info.size.unwrap_or(0);
            let block_size = manifest.block_size.unwrap_or(4096) as u64;
            let size_in_blocks = size_in_bytes / block_size;
            let total_blocks = size_in_bytes / block_size;
            let hash = info.hash.as_ref().map(hex::encode);
            let mut start_offset = data_offset;
            for op in &partition.operations {
                if let Some(_first_extent) = op.dst_extents.first() {
                    //2
                    start_offset = data_offset + op.data_offset.unwrap_or(0);
                    break;
                }
            }
            let end_offset = start_offset + size_in_bytes;
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
                partition_names: group.partition_names.clone(),
            })
            .collect();

        let vabc_feature_set = dpm.vabc_feature_set.as_ref().map(|fs| VabcFeatureSetInfo {
            threaded: fs.threaded,
            batch_writes: fs.batch_writes,
        });

        Some(DynamicPartitionInfo {
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
        })
        .collect();

    let payload_metadata = PayloadMetadata {
        security_patch_level: manifest.security_patch_level.clone(),
        block_size: manifest.block_size.unwrap_or(4096),
        minor_version: manifest.minor_version.unwrap_or(0),
        max_timestamp: manifest.max_timestamp,
        dynamic_partition_metadata,
        partial_update: manifest.partial_update,
        apex_info,
        partitions,
    };

    let json = serde_json::to_string_pretty(&payload_metadata)?;

    if output_dir.to_string_lossy() == "-" {
        return Ok(json);
    }

    let metadata_path = output_dir.join("payload_metadata.json");
    fs::write(metadata_path, &json)?;

    Ok(json)
}
