// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use anyhow::Result;
use async_trait::async_trait;

/// abstract I/O trait for reading ZIP files from any source
#[async_trait]
pub trait ZipIO: Send + Sync {
    /// read exact number of bytes at given offset
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()>;

    /// get total size of the source
    async fn size(&self) -> Result<u64>;
}
