// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

mod cli;
use cli::entry::run;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{:#?}", e);
        std::process::exit(1);
    }
}
