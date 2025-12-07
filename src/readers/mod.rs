// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust
//
// This file is part of payload-dumper-rust. It implements components used for
// extracting and processing Android OTA payloads.

pub mod local_reader;
#[cfg(feature = "local_zip")]
pub mod local_zip_reader;
#[cfg(feature = "remote_zip")]
pub mod remote_bin_reader;
#[cfg(feature = "remote_zip")]
pub mod remote_zip_reader;
