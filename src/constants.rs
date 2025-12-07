// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust
//
// This file is part of payload-dumper-rust. It implements components used for
// extracting and processing Android OTA payloads.

// ZIP signatures
pub const ZIP_MAGIC: [u8; 2] = [0x50, 0x4B];

#[cfg(feature = "local_zip")]
pub const LOCAL_FILE_HEADER_SIGNATURE: [u8; 4] = [ZIP_MAGIC[0], ZIP_MAGIC[1], 0x03, 0x04];

#[cfg(feature = "local_zip")]
pub const CENTRAL_DIR_HEADER_SIGNATURE: [u8; 4] = [ZIP_MAGIC[0], ZIP_MAGIC[1], 0x01, 0x02];

#[cfg(feature = "local_zip")]
pub const EOCD_SIGNATURE: [u8; 4] = [ZIP_MAGIC[0], ZIP_MAGIC[1], 0x05, 0x06];

#[cfg(feature = "local_zip")]
pub const ZIP64_EOCD_SIGNATURE: [u8; 4] = [ZIP_MAGIC[0], ZIP_MAGIC[1], 0x06, 0x06];

#[cfg(feature = "local_zip")]
pub const ZIP64_EOCD_LOCATOR_SIGNATURE: [u8; 4] = [ZIP_MAGIC[0], ZIP_MAGIC[1], 0x06, 0x07];

#[cfg(feature = "remote_zip")]
pub const DEFAULT_USER_AGENT: &str = env!("DEFAULT_USER_AGENT");

// Payload
pub const PAYLOAD_MAGIC: &[u8; 4] = b"CrAU";
pub const SUPPORTED_PAYLOAD_VERSION: u64 = 2;
