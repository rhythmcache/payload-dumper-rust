// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use crate::payload::payload_dumper::{AsyncPayloadRead, PayloadReader};
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::fs::File;
use tokio::io::AsyncSeekExt;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};

pub struct LocalAsyncZipPayloadReader {
    path: PathBuf,
    payload_offset: u64,
}

impl LocalAsyncZipPayloadReader {
    pub async fn new(zip_path: PathBuf) -> Result<Self> {
        // use LocalZipIO for parsing
        let io = crate::zip::local_zip_io::LocalZipIO::new(zip_path.clone()).await?;

        // find payload.bin entry
        let entry = crate::zip::core_parser::ZipParser::find_payload_entry(&io).await?;

        // get actual data offset (after local header)
        let data_offset = crate::zip::core_parser::ZipParser::get_data_offset(&io, &entry).await?;

        // verify it's actually a payload file
        crate::zip::core_parser::ZipParser::verify_payload_magic(&io, data_offset).await?;

        Ok(Self {
            path: zip_path,
            payload_offset: data_offset,
        })
    }
}

#[async_trait]
impl AsyncPayloadRead for LocalAsyncZipPayloadReader {
    async fn open_reader(&self) -> Result<Box<dyn PayloadReader>> {
        let file = File::open(&self.path).await?;
        Ok(Box::new(LocalZipPayloadReader {
            file: BufReader::new(file),
            payload_offset: self.payload_offset,
        }))
    }
}

struct LocalZipPayloadReader {
    file: BufReader<File>,
    payload_offset: u64,
}

#[async_trait]
impl PayloadReader for LocalZipPayloadReader {
    async fn read_range(
        &mut self,
        offset: u64,
        length: u64,
    ) -> Result<Pin<Box<dyn AsyncRead + Send + '_>>> {
        // offset is relative to payload.bin start, add ZIP offset
        let absolute_offset = self.payload_offset + offset;
        self.file
            .seek(std::io::SeekFrom::Start(absolute_offset))
            .await?;
        Ok(Box::pin((&mut self.file).take(length)))
    }
}
