use clap::Parser;
use std::path::PathBuf;

const VERSION_STRING: &str = concat!(
    env!("CARGO_PKG_VERSION"), "\n\n",
    "Copyright (C) 2024-2025 rhythmcache\n",
    "License Apache-2.0: Apache License 2.0 <https://www.apache.org/licenses/LICENSE-2.0>\n",
    "\n",
    "This is free software; you are free to change and redistribute it.\n",
    "There is NO WARRANTY, to the extent permitted by law.\n",
    "\n",
    "Project home: <https://github.com/rhythmcache/payload-dumper-rust>\n",
    "\n",
    "Build Information:\n",
    "  Version:    ", env!("CARGO_PKG_VERSION"), "\n",
    "  Git:        ", env!("GIT_COMMIT_SHORT"), " (", env!("GIT_BRANCH"), ")", "\n",
    "  Built:      ", env!("BUILD_TIMESTAMP"), "\n",
    "  Rustc:      ", env!("RUSTC_VERSION"), "\n",
    "  Host:       ", env!("BUILD_HOST"), "\n",
    "\n",
    "Target Information:\n",
    "  Target:     ", env!("BUILD_TARGET"), "\n",
    "  Arch:       ", env!("TARGET_ARCH"), "\n",
    "  OS:         ", env!("TARGET_OS"), "\n",
    "\n",
    "Build Configuration:\n",
    "  Profile:    ", env!("BUILD_PROFILE"), "\n",
    "  Opt Level:  ", env!("OPT_LEVEL"), "\n",
    "  Features:   ", env!("BUILD_FEATURES"), "\n"
);

#[derive(Parser)]
#[command(
    version = VERSION_STRING,
    about = "A fast and efficient Android OTA payload dumper written in Rust"
)]
#[command(next_line_help = true)]
pub struct Args {
    pub payload_path: PathBuf,

    #[arg(
        short = 'o',
        long,
        default_value = "output",
        help = "Output directory for extracted partitions"
    )]
    pub out: PathBuf,

    #[arg(
        short = 'U',
        long,
        help = "Custom User-Agent string for HTTP requests (only used with remote URLs)"
    )]
    pub user_agent: Option<String>,

    #[cfg(feature = "differential_ota")]
    #[arg(short = 'd', long, help = "Enable differential OTA mode (requires --old)")]
    pub diff: bool,

    #[cfg(feature = "differential_ota")]
    #[arg(
        short = 'O',
        long,
        default_value = "old",
        help = "Path to directory containing old partition images (required for --diff)"
    )]
    pub old: PathBuf,

    #[arg(
        short = 'i',
        long,
        default_value = "",
        alias = "partitions",
        hide_default_value = true,
        help = "Comma-separated list of partition names to extract"
    )]
    pub images: String,

    #[arg(
        short = 't',
        long,
        alias = "concurrency",
        help = "Number of threads to use for parallel processing"
    )]
    pub threads: Option<usize>,

    #[cfg(feature = "differential_ota")]
    #[arg(
        short = 'l',
        long,
        conflicts_with_all = &["diff", "old", "images", "threads"],
        help = "List available partitions in the payload"
    )]
    pub list: bool,

    #[cfg(not(feature = "differential_ota"))]
    #[arg(
        short = 'l',
        long,
        conflicts_with_all = &["images", "threads"],
        help = "List available partitions in the payload"
    )]
    pub list: bool,

    #[cfg(feature = "differential_ota")]
    #[arg(
        short = 'm',
        long,
        help = "Save complete metadata as JSON (use --out - to write to stdout)",
        conflicts_with_all = &["diff", "old", "images"],
        hide = cfg!(not(feature = "metadata"))
    )]
    pub metadata: bool,

    #[cfg(not(feature = "differential_ota"))]
    #[arg(
        short = 'm',
        long,
        help = "Save complete metadata as JSON (use --out - to write to stdout)",
        conflicts_with_all = &["images"],
        hide = cfg!(not(feature = "metadata"))
    )]
    pub metadata: bool,

    #[arg(short = 'P', long, help = "Disable parallel extraction")]
    pub no_parallel: bool,

    #[arg(short = 'n', long, help = "Skip hash verification")]
    pub no_verify: bool,
}
