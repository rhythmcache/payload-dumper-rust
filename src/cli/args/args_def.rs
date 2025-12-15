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
        help = if cfg!(feature = "remote_zip") {
            "Custom User-Agent string for HTTP requests (only used with remote URLs)"
        } else {
            "Custom User-Agent string for HTTP requests [requires remote_zip feature]"
        },
        hide = cfg!(not(feature = "remote_zip"))
    )]
    pub user_agent: Option<String>,

    #[arg(
        short = 'C',
    long,
    help = if cfg!(feature = "remote_zip") {
        "Custom HTTP Cookie header value for remote requests (e.g. \"key1=value1; key2=value2\")"
    } else {
        "Custom HTTP Cookie header value [requires remote_zip feature]"
    },
    hide = cfg!(not(feature = "remote_zip"))
)]
    pub cookies: Option<String>,

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

    #[arg(
        short = 'l',
        long,
        conflicts_with_all = &["images", "threads"],
        help = "List available partitions in the payload"
    )]
    pub list: bool,

    #[arg(
    short = 'm',
    long,
    value_name = "MODE",
    num_args = 0..=1,
    default_missing_value = "compact",
    require_equals = true,
    help = "Save metadata as JSON. Use '--metadata=full' for detailed info including all operations",
    long_help = "Save metadata as JSON:\n  \
                 --metadata        Compact mode (default, ~100KB)\n  \
                 --metadata=full   Full mode with all operation details (may be large)\n  \
                 Can be combined with --images to export metadata for specific partitions only",
)]
    pub metadata: Option<String>,

    #[arg(short = 'P', long, help = "Disable parallel extraction")]
    pub no_parallel: bool,

    #[arg(short = 'n', long, help = "Skip hash verification")]
    pub no_verify: bool,

    #[arg(
        long,
        help = "Pre-download all partition data before extraction (only for remote URLs)",
        long_help = "For remote URLs, download all required partition data to a temporary directory \
                     before extraction. This eliminates per-operation network latency at the cost of \
                     upfront download time. Most effective for slow/high-latency connections.",
        hide = cfg!(not(feature = "prefetch"))
    )]
    pub prefetch: bool,

    #[arg(
        short = 'q',
        long,
        help = "Suppress all non-essential output (errors will still be shown)"
    )]
    pub quiet: bool,
}
