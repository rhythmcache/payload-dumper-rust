use clap::Parser;
use serde::Serialize;
use std::path::PathBuf;


#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(next_line_help = true)]

pub struct Args {
    pub payload_path: PathBuf,

    #[arg(
        long,
        default_value = "output",
        help = "Output directory for extracted partitions"
    )]
    pub out: PathBuf,

    #[cfg(feature = "differential_ota")]
    #[arg(long, help = "Enable differential OTA mode (requires --old)")]
    pub diff: bool,

    #[cfg(feature = "differential_ota")]
    #[arg(
        long,
        default_value = "old",
        help = "Path to the directory containing old partition images (required for --diff)"
    )]
    pub old: PathBuf,

    #[arg(
        long,
        default_value = "",
        hide_default_value = true,
        help = "Comma-separated list of partition names to extract"
    )]
    pub images: String,

    #[arg(long, help = "Number of threads to use for parallel processing")]
    pub threads: Option<usize>,

    #[cfg(feature = "differential_ota")]
    #[arg(
        long,
        conflicts_with_all = &["diff", "old", "images", "threads"],
        help = "List available partitions in the payload"
    )]
    pub list: bool,

    #[cfg(not(feature = "differential_ota"))]
    #[arg(
        long,
        conflicts_with_all = &["images", "threads"],
        help = "List available partitions in the payload"
    )]
    pub list: bool,

    #[cfg(feature = "differential_ota")]
    #[arg(
        long,
        help = "Save Complete Metadata as JSON ( use --out - to write to stdout)",
        conflicts_with_all = &["diff", "old", "images"]
    )]
    pub metadata: bool,

    #[cfg(not(feature = "differential_ota"))]
    #[arg(
        long,
        help = "Save Complete Metadata as JSON ( use --out - to write to stdout)",
        conflicts_with_all = &["images"]
    )]
    pub metadata: bool,

    #[arg(long, help = "Disable parallel extraction")]
    pub no_parallel: bool,

    #[arg(long, help = "Skip hash verification")]
    pub no_verify: bool,
}


#[derive(Serialize)]
pub struct PartitionMetadata {
    pub partition_name: String,
    pub size_in_blocks: u64,
    pub size_in_bytes: u64,
    pub size_readable: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    pub start_offset: u64,
    pub end_offset: u64,
    pub data_offset: u64,
    pub partition_type: String,
    pub operations_count: usize,
    pub compression_type: String,
    pub encryption: String,
    pub block_size: u64,
    pub total_blocks: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_postinstall: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postinstall_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postinstall_optional: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash_tree_algorithm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct DynamicPartitionGroupInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub partition_names: Vec<String>,
}

#[derive(Serialize)]
pub struct VabcFeatureSetInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threaded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_writes: Option<bool>,
}

#[derive(Serialize)]
pub struct DynamicPartitionInfo {
    pub groups: Vec<DynamicPartitionGroupInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vabc_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vabc_compression_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cow_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vabc_feature_set: Option<VabcFeatureSetInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_factor: Option<u64>,
}

#[derive(Serialize)]
pub struct ApexInfoMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_compressed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decompressed_size: Option<i64>,
}

#[derive(Serialize)]
pub struct PayloadMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_patch_level: Option<String>,
    pub block_size: u32,
    pub minor_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_timestamp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_partition_metadata: Option<DynamicPartitionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_update: Option<bool>,
    pub apex_info: Vec<ApexInfoMetadata>,
    pub partitions: Vec<PartitionMetadata>,
}
