// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use clap::Parser;
use std::path::PathBuf;

const VERSION_STRING: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\n\n",
    "Copyright (C) 2024-2025 rhythmcache\n",
    "License Apache-2.0: Apache License 2.0 <https://www.apache.org/licenses/LICENSE-2.0>\n",
    "\n",
    "This is free software; you are free to change and redistribute it.\n",
    "There is NO WARRANTY, to the extent permitted by law.\n",
    "\n",
    "Project home: <https://github.com/rhythmcache/payload-dumper-rust>\n",
    "\n",
    "Build Information:\n",
    "  Version:    ",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "  Git:        ",
    env!("GIT_COMMIT_SHORT"),
    " (",
    env!("GIT_BRANCH"),
    ")",
    "\n",
    "  Built:      ",
    env!("BUILD_TIMESTAMP"),
    "\n",
    "  Rustc:      ",
    env!("RUSTC_VERSION"),
    "\n",
    "  Host:       ",
    env!("BUILD_HOST"),
    "\n",
    "\n",
    "Target Information:\n",
    "  Target:     ",
    env!("BUILD_TARGET"),
    "\n",
    "  Arch:       ",
    env!("TARGET_ARCH"),
    "\n",
    "  OS:         ",
    env!("TARGET_OS"),
    "\n",
    "\n",
    "Build Configuration:\n",
    "  Profile:    ",
    env!("BUILD_PROFILE"),
    "\n",
    "  Opt Level:  ",
    env!("OPT_LEVEL"),
    "\n",
    "  Features:   ",
    env!("BUILD_FEATURES"),
    "\n"
);

#[derive(Parser)]
#[command(
    version = VERSION_STRING,
    about = "A fast and efficient Android OTA payload dumper"
)]
#[command(next_line_help = true)]
pub struct Args {
    #[arg(
        value_name = "PAYLOAD",
        help = "Path to payload file or remote URL",
        long_help = "Path to the Android OTA payload file. Can be a local path to a .bin file, \
                     a .zip archive containing payload.bin, or a remote URL to download from"
    )]
    pub payload_path: PathBuf,

    #[arg(
        short = 'o',
        long,
        default_value = "output",
        value_name = "DIR",
        help = "Directory to save extracted partitions"
    )]
    pub out: PathBuf,

    #[arg(
        long,
        default_value = "old",
        value_name = "DIR",
        help = "Directory containing source images for differential OTA",
        long_help = "Path to directory containing the old partition images. Required for differential \
                     (incremental) OTA updates that contain only the changes from a previous version. \
                     The tool applies these delta operations to the old images to generate new ones. \
                     Not needed for full OTA updates"
    )]
    pub source_dir: PathBuf,

    #[arg(
        short = 'U',
        long,
        value_name = "AGENT",
        help = if cfg!(feature = "remote_zip") {
            "Custom User-Agent for HTTP requests"
        } else {
            "Custom User-Agent for HTTP requests [requires remote_zip feature]"
        },
        long_help = if cfg!(feature = "remote_zip") {
            "Custom User-Agent string to identify as when making HTTP requests. Some servers may \
             block or rate-limit requests based on User-Agent, or require specific browser \
             identification to serve files"
        } else {
            "Custom User-Agent string for HTTP requests. This feature requires compilation \
             with --features remote_zip"
        },
        hide = cfg!(not(feature = "remote_zip"))
    )]
    pub user_agent: Option<String>,

    #[arg(
        short = 'C',
        long,
        value_name = "COOKIES",
        help = if cfg!(feature = "remote_zip") {
            "HTTP Cookie header for authenticated requests"
        } else {
            "HTTP Cookie header [requires remote_zip feature]"
        },
        long_help = if cfg!(feature = "remote_zip") {
            "Custom HTTP Cookie header value for requests that require authentication or session \
             management. Needed when downloading from servers that gate access behind login sessions \
             or require specific cookie values for authorization"
        } else {
            "HTTP Cookie header for authenticated requests. This feature requires compilation \
             with --features remote_zip"
        },
        hide = cfg!(not(feature = "remote_zip"))
    )]
    pub cookies: Option<String>,

    #[arg(
        short = 'i',
        long,
        default_value = "",
        alias = "partitions",
        value_name = "NAMES",
        hide_default_value = true,
        help = "Comma-separated list of partitions to extract",
        long_help = "Extract only specific partitions instead of all available ones. \
                     Provide partition names as a comma-separated list. Use --list to see \
                     available partition names in the payload"
    )]
    pub images: String,

    #[arg(
        short = 't',
        long,
        alias = "concurrency",
        value_name = "COUNT",
        help = "Number of threads for parallel extraction",
        long_help = "Number of worker threads to use for concurrent partition extraction. \
                     More threads can significantly speed up extraction on systems with fast \
                     storage, but will use more memory and CPU resources. Automatically defaults \
                     to twice the number of CPU cores, capped at 32"
    )]
    pub threads: Option<usize>,

    #[arg(
        short = 'l',
        long,
        conflicts_with_all = &["images", "threads"],
        help = "List available partitions and exit",
        long_help = "Display all partitions present in the payload with their sizes and types, \
                     then exit without extracting. Useful for inspecting OTA contents before \
                     deciding which partitions to extract"
    )]
    pub list: bool,

    #[arg(
        short = 'm',
        long,
        value_name = "MODE",
        num_args = 0..=1,
        default_missing_value = "compact",
        require_equals = true,
        help = "Save payload metadata as JSON",
        long_help = "Export payload metadata to a JSON file. Compact mode includes essential \
                     information like partition names, sizes, and hashes. Full mode additionally \
                     includes all low-level operation details, which can be very large but useful \
                     for debugging or analysis. Can be combined with --images to export metadata \
                     only for specific partitions"
    )]
    pub metadata: Option<String>,

    #[arg(
        short = 'P',
        long,
        help = "Disable parallel extraction",
        long_help = "Process partitions sequentially instead of in parallel. Reduces memory usage \
                     and CPU load at the cost of slower extraction time. Useful on resource-constrained \
                     systems or when running alongside other intensive tasks"
    )]
    pub no_parallel: bool,

    #[arg(
        short = 'n',
        long,
        help = "Skip hash verification of extracted partitions",
        long_help = "Skip cryptographic hash verification after extraction. Verification ensures \
                     extracted partitions match the expected checksums from the payload manifest. \
                     Skipping saves time but risks using corrupted data if extraction or download errors occurred"
    )]
    pub no_verify: bool,

    #[arg(
        long,
        help = "Pre-download all data before extraction (remote URLs only)",
        long_help = "Download all required partition data to a temporary directory before starting \
                     extraction. This eliminates network latency during per-operation processing, \
                     trading upfront download time for faster overall extraction. Most beneficial \
                     on slow or high-latency network connections. Only applicable to remote URLs",
        hide = cfg!(not(feature = "prefetch"))
    )]
    pub prefetch: bool,

    #[arg(
        short = 'q',
        long,
        help = "Suppress non-essential output",
        long_help = "Reduce output verbosity by suppressing progress indicators and informational \
                     messages. Errors and warnings will still be displayed."
    )]
    pub quiet: bool,
}
