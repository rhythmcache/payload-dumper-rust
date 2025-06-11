use clap::Parser;
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
        conflicts_with_all = &["diff", "old", "images"],
        hide = cfg!(not(feature = "metadata"))
    )]
    pub metadata: bool,

    #[cfg(not(feature = "differential_ota"))]
    #[arg(
        long,
        help = "Save Complete Metadata as JSON ( use --out - to write to stdout)",
        conflicts_with_all = &["images"],
        hide = cfg!(not(feature = "metadata"))
    )]
    pub metadata: bool,

    #[arg(long, help = "Disable parallel extraction")]
    pub no_parallel: bool,

    #[arg(long, help = "Skip hash verification")]
    pub no_verify: bool,
}