use crate::InstallOperation;
pub use crate::PartitionUpdate;
use crate::ReadSeek;
use crate::install_operation;
use crate::module::args::Args;
#[cfg(feature = "differential_ota")]
use crate::module::patch::bspatch;
#[cfg(feature = "differential_ota")]
use crate::module::verify::verify_old_partition;
#[cfg(feature = "differential_ota")]
use anyhow::Context;
use anyhow::{Result, anyhow};
use bzip2::read::BzDecoder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::Duration;

// Enum to track the payload source type
#[derive(Clone, Copy, Debug)]
pub enum PayloadSource {
    LocalFile,
    RemoteHttp,
    LocalZip,
    RemoteZip,
}

impl PayloadSource {
    pub fn should_use_buffered_approach(&self) -> bool {
        match self {
            PayloadSource::LocalFile => false, // Use streaming for local files
            PayloadSource::RemoteHttp => true, // Use buffered for HTTP
            PayloadSource::LocalZip => false,  // Use streaming for local ZIP
            PayloadSource::RemoteZip => true,  // Use buffered for remote ZIP
        }
    }
}

// Streaming approach - good for local files
pub fn process_operation_streaming(
    operation_index: usize,
    op: &InstallOperation,
    data_offset: u64,
    block_size: u64,
    payload_file: &mut (impl Read + Seek),
    out_file: &mut (impl Write + Seek),
    #[allow(unused_variables)] old_file: Option<&mut dyn ReadSeek>,
) -> Result<()> {
    if op.dst_extents.is_empty() {
        return Err(anyhow!(
            "Invalid operation.dst_extents for operation {}",
            operation_index
        ));
    }

    let dst_extent = &op.dst_extents[0];
    let data_length = op.data_length.unwrap_or(0);
    let expected_uncompressed_size = dst_extent.num_blocks.unwrap_or(0) * block_size;

    // Seek to the data position in payload file
    payload_file.seek(SeekFrom::Start(data_offset + op.data_offset.unwrap_or(0)))?;

    // Seek to the output position
    out_file.seek(SeekFrom::Start(
        dst_extent.start_block.unwrap_or(0) * block_size,
    ))?;

    // Create a limited reader for the operation data
    let mut limited_reader = payload_file.take(data_length);

    match op.r#type() {
        install_operation::Type::Replace => {
            let bytes_written = io::copy(&mut limited_reader, out_file)?;
            if bytes_written != expected_uncompressed_size {
                return Err(anyhow!(
                    "Verify failed (Unexpected bytes written): operation {} ({} != {})",
                    operation_index,
                    bytes_written,
                    expected_uncompressed_size
                ));
            }
        }

        install_operation::Type::ReplaceXz => {
            let mut xz_decoder = liblzma::read::XzDecoder::new(limited_reader);
            match io::copy(&mut xz_decoder, out_file) {
                Ok(bytes_written) => {
                    if bytes_written != expected_uncompressed_size {
                        return Err(anyhow!(
                            "Verify failed (Unexpected bytes written): operation {} ({} != {})",
                            operation_index,
                            bytes_written,
                            expected_uncompressed_size
                        ));
                    }
                }
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to XZ decompression error: {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }

        install_operation::Type::ReplaceBz => {
            let mut bz_decoder = BzDecoder::new(limited_reader);
            match io::copy(&mut bz_decoder, out_file) {
                Ok(bytes_written) => {
                    if bytes_written != expected_uncompressed_size {
                        return Err(anyhow!(
                            "Verify failed (Unexpected bytes written): operation {} ({} != {})",
                            operation_index,
                            bytes_written,
                            expected_uncompressed_size
                        ));
                    }
                }
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to BZ2 decompression error: {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }

        install_operation::Type::Zstd => match zstd::Decoder::new(limited_reader) {
            Ok(mut zstd_decoder) => match io::copy(&mut zstd_decoder, out_file) {
                Ok(bytes_written) => {
                    if bytes_written != expected_uncompressed_size {
                        return Err(anyhow!(
                            "Verify failed (Unexpected bytes written): operation {} ({} != {})",
                            operation_index,
                            bytes_written,
                            expected_uncompressed_size
                        ));
                    }
                }
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to ZSTD decompression error: {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            },
            Err(e) => {
                println!(
                    "  Warning: Skipping operation {} due to ZSTD decoder creation error: {}",
                    operation_index, e
                );
                return Ok(());
            }
        },

        install_operation::Type::Zero => {
            let zero_reader = io::repeat(0).take(expected_uncompressed_size);
            let bytes_written =
                io::copy(&mut zero_reader.take(expected_uncompressed_size), out_file)?;
            if bytes_written != expected_uncompressed_size {
                return Err(anyhow!(
                    "Verify failed (Unexpected bytes written): operation {} ({} != {})",
                    operation_index,
                    bytes_written,
                    expected_uncompressed_size
                ));
            }
        }

        install_operation::Type::SourceCopy => {
            #[cfg(feature = "differential_ota")]
            {
                let old_file = old_file
                    .ok_or_else(|| anyhow!("SOURCE_COPY requires an old file to copy from"))?;
                for ext in &op.src_extents {
                    old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                    let mut limited_old_reader =
                        old_file.take(ext.num_blocks.unwrap_or(0) * block_size);
                    io::copy(&mut limited_old_reader, out_file)?;
                }
            }
            #[cfg(not(feature = "differential_ota"))]
            {
                return Err(anyhow!(
                    "Operation {} (SOURCE_COPY) requires differential_ota feature to be enabled",
                    operation_index
                ));
            }
        }

        #[cfg(feature = "differential_ota")]
        install_operation::Type::SourceBsdiff => {
            let old_file = old_file
                .ok_or_else(|| anyhow!("SOURCE_BSDIFF requires differential OTA support"))?;

            // Read patch data into memory (streaming not practical for bsdiff)
            let mut patch_data = Vec::new();
            limited_reader.read_to_end(&mut patch_data)?;

            let mut old_data = Vec::new();
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                old_data.extend_from_slice(&buffer);
            }

            let new_data = bspatch(&old_data, &patch_data)?;
            out_file.write_all(&new_data)?;
        }

        #[cfg(feature = "differential_ota")]
        install_operation::Type::BrotliBsdiff => {
            let old_file = old_file
                .ok_or_else(|| anyhow!("BROTLI_BSDIFF requires differential OTA support"))?;

            // Read and decompress patch data
            let mut compressed_patch = Vec::new();
            limited_reader.read_to_end(&mut compressed_patch)?;

            let mut decompressed_patch = Vec::new();
            brotli::Decompressor::new(&compressed_patch[..], 4096)
                .read_to_end(&mut decompressed_patch)?;

            let mut old_data = Vec::new();
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                old_data.extend_from_slice(&buffer);
            }

            let new_data = bspatch(&old_data, &decompressed_patch)?;
            out_file.write_all(&new_data)?;
        }

        #[cfg(feature = "differential_ota")]
        install_operation::Type::Lz4diffBsdiff => {
            let old_file = old_file
                .ok_or_else(|| anyhow!("LZ4DIFF_BSDIFF requires differential OTA support"))?;

            // Read and decompress patch data
            let mut compressed_patch = Vec::new();
            limited_reader.read_to_end(&mut compressed_patch)?;

            let decompressed_patch = lz4_flex::block::decompress_size_prepended(&compressed_patch)?;

            let mut old_data = Vec::new();
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                old_data.extend_from_slice(&buffer);
            }

            let new_data = bspatch(&old_data, &decompressed_patch)?;
            out_file.write_all(&new_data)?;
        }

        #[cfg(not(feature = "differential_ota"))]
        install_operation::Type::SourceBsdiff
        | install_operation::Type::BrotliBsdiff
        | install_operation::Type::Lz4diffBsdiff => {
            return Err(anyhow!(
                "Operation {} requires differential_ota feature to be enabled",
                operation_index
            ));
        }

        _ => {
            println!(
                "  Warning: Skipping operation {} due to unknown compression method",
                operation_index
            );
            return Ok(());
        }
    }

    Ok(())
}

// Buffered approach - good for remote HTTP files
pub fn process_operation_buffered(
    operation_index: usize,
    op: &InstallOperation,
    data_offset: u64,
    block_size: u64,
    payload_file: &mut (impl Read + Seek),
    out_file: &mut (impl Write + Seek),
    #[allow(unused_variables)] old_file: Option<&mut dyn ReadSeek>,
) -> Result<()> {
    if op.dst_extents.is_empty() {
        return Err(anyhow!(
            "Invalid operation.dst_extents for operation {}",
            operation_index
        ));
    }

    // Seek to the data position and read all data into memory
    payload_file.seek(SeekFrom::Start(data_offset + op.data_offset.unwrap_or(0)))?;
    let mut data = vec![0u8; op.data_length.unwrap_or(0) as usize];
    payload_file.read_exact(&mut data)?;

    match op.r#type() {
        install_operation::Type::Replace => {
            // For uncompressed data, write to all destination extents
            let mut pos = 0;
            for ext in &op.dst_extents {
                let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                let end_pos = pos + ext_size;
                if end_pos <= data.len() {
                    out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                    out_file.write_all(&data[pos..end_pos])?;
                    pos = end_pos;
                }
            }
        }

        install_operation::Type::ReplaceXz => match liblzma::decode_all(Cursor::new(&data)) {
            Ok(decompressed) => {
                let mut pos = 0;
                for ext in &op.dst_extents {
                    let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                    let end_pos = pos + ext_size;
                    if end_pos <= decompressed.len() {
                        out_file
                            .seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                        out_file.write_all(&decompressed[pos..end_pos])?;
                        pos = end_pos;
                    }
                }
            }
            Err(e) => {
                println!(
                    "  Warning: Skipping operation {} due to XZ decompression error: {}",
                    operation_index, e
                );
                return Ok(());
            }
        },

        install_operation::Type::ReplaceBz => {
            let mut bz_decoder = BzDecoder::new(Cursor::new(&data));
            let mut decompressed = Vec::new();
            match bz_decoder.read_to_end(&mut decompressed) {
                Ok(_) => {
                    let mut pos = 0;
                    for ext in &op.dst_extents {
                        let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                        let end_pos = pos + ext_size;
                        if end_pos <= decompressed.len() {
                            out_file
                                .seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                            out_file.write_all(&decompressed[pos..end_pos])?;
                            pos = end_pos;
                        }
                    }
                }
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to BZ2 decompression error: {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }

        install_operation::Type::Zstd => match zstd::decode_all(Cursor::new(&data)) {
            Ok(decompressed) => {
                let mut pos = 0;
                for ext in &op.dst_extents {
                    let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                    let end_pos = pos + ext_size;
                    if end_pos <= decompressed.len() {
                        out_file
                            .seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                        out_file.write_all(&decompressed[pos..end_pos])?;
                        pos = end_pos;
                    }
                }
            }
            Err(e) => {
                println!(
                    "  Warning: Skipping operation {} due to ZSTD decompression error: {}",
                    operation_index, e
                );
                return Ok(());
            }
        },

        install_operation::Type::Zero => {
            for ext in &op.dst_extents {
                let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                let zero_data = vec![0u8; ext_size];
                out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                out_file.write_all(&zero_data)?;
            }
        }

        install_operation::Type::SourceCopy => {
            #[cfg(feature = "differential_ota")]
            {
                let old_file = old_file
                    .ok_or_else(|| anyhow!("SOURCE_COPY requires an old file to copy from"))?;
                for (src_ext, dst_ext) in op.src_extents.iter().zip(op.dst_extents.iter()) {
                    let ext_size = (src_ext.num_blocks.unwrap_or(0) * block_size) as usize;
                    let mut buffer = vec![0u8; ext_size];

                    old_file.seek(SeekFrom::Start(
                        src_ext.start_block.unwrap_or(0) * block_size,
                    ))?;
                    old_file.read_exact(&mut buffer)?;

                    out_file.seek(SeekFrom::Start(
                        dst_ext.start_block.unwrap_or(0) * block_size,
                    ))?;
                    out_file.write_all(&buffer)?;
                }
            }
            #[cfg(not(feature = "differential_ota"))]
            {
                return Err(anyhow!(
                    "Operation {} (SOURCE_COPY) requires differential_ota feature to be enabled",
                    operation_index
                ));
            }
        }

        #[cfg(feature = "differential_ota")]
        install_operation::Type::SourceBsdiff => {
            let old_file = old_file
                .ok_or_else(|| anyhow!("SOURCE_BSDIFF requires differential OTA support"))?;

            let mut old_data = Vec::new();
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                old_data.extend_from_slice(&buffer);
            }

            let new_data = bspatch(&old_data, &data)?;

            let mut pos = 0;
            for ext in &op.dst_extents {
                let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                let end_pos = pos + ext_size;
                if end_pos <= new_data.len() {
                    out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                    out_file.write_all(&new_data[pos..end_pos])?;
                    pos = end_pos;
                }
            }
        }

        #[cfg(feature = "differential_ota")]
        install_operation::Type::BrotliBsdiff => {
            let old_file = old_file
                .ok_or_else(|| anyhow!("BROTLI_BSDIFF requires differential OTA support"))?;

            let mut decompressed_patch = Vec::new();
            brotli::Decompressor::new(&data[..], 4096).read_to_end(&mut decompressed_patch)?;

            let mut old_data = Vec::new();
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                old_data.extend_from_slice(&buffer);
            }

            let new_data = bspatch(&old_data, &decompressed_patch)?;

            let mut pos = 0;
            for ext in &op.dst_extents {
                let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                let end_pos = pos + ext_size;
                if end_pos <= new_data.len() {
                    out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                    out_file.write_all(&new_data[pos..end_pos])?;
                    pos = end_pos;
                }
            }
        }

        #[cfg(feature = "differential_ota")]
        install_operation::Type::Lz4diffBsdiff => {
            let old_file = old_file
                .ok_or_else(|| anyhow!("LZ4DIFF_BSDIFF requires differential OTA support"))?;

            let decompressed_patch = lz4_flex::block::decompress_size_prepended(&data)?;

            let mut old_data = Vec::new();
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                old_data.extend_from_slice(&buffer);
            }

            let new_data = bspatch(&old_data, &decompressed_patch)?;

            let mut pos = 0;
            for ext in &op.dst_extents {
                let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                let end_pos = pos + ext_size;
                if end_pos <= new_data.len() {
                    out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                    out_file.write_all(&new_data[pos..end_pos])?;
                    pos = end_pos;
                }
            }
        }

        #[cfg(not(feature = "differential_ota"))]
        install_operation::Type::SourceBsdiff
        | install_operation::Type::BrotliBsdiff
        | install_operation::Type::Lz4diffBsdiff => {
            return Err(anyhow!(
                "Operation {} requires differential_ota feature to be enabled",
                operation_index
            ));
        }

        _ => {
            println!(
                "  Warning: Skipping operation {} due to unknown compression method",
                operation_index
            );
            return Ok(());
        }
    }

    Ok(())
}

// Unified function that chooses the appropriate method
pub fn process_operation(
    operation_index: usize,
    op: &InstallOperation,
    data_offset: u64,
    block_size: u64,
    payload_file: &mut (impl Read + Seek),
    out_file: &mut (impl Write + Seek),
    old_file: Option<&mut dyn ReadSeek>,
    payload_source: PayloadSource,
) -> Result<()> {
    if payload_source.should_use_buffered_approach() {
        process_operation_buffered(
            operation_index,
            op,
            data_offset,
            block_size,
            payload_file,
            out_file,
            old_file,
        )
    } else {
        process_operation_streaming(
            operation_index,
            op,
            data_offset,
            block_size,
            payload_file,
            out_file,
            old_file,
        )
    }
}

pub fn dump_partition(
    partition: &PartitionUpdate,
    data_offset: u64,
    block_size: u64,
    args: &Args,
    payload_file: &mut (impl Read + Seek),
    multi_progress: Option<&MultiProgress>,
    payload_source: PayloadSource,
) -> Result<()> {
    let partition_name = &partition.partition_name;
    let total_ops = partition.operations.len() as u64;
    let progress_bar = if let Some(mp) = multi_progress {
        let pb = mp.add(ProgressBar::new(100));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/white}] {percent}% - {msg}")
            .unwrap()
            .progress_chars("▰▱"));
        pb.enable_steady_tick(Duration::from_millis(500));
        pb.set_message(format!(
            "Processing {} ({} ops) [{}]",
            partition_name,
            total_ops,
            if payload_source.should_use_buffered_approach() {
                "buffered"
            } else {
                "streaming"
            }
        ));
        Some(pb)
    } else {
        None
    };

    let out_dir = &args.out;
    if args.out.to_string_lossy() != "-" {
        fs::create_dir_all(out_dir)?;
    }
    let out_path = out_dir.join(format!("{}.img", partition_name));
    let mut out_file = File::create(&out_path)?;

    if let Some(info) = &partition.new_partition_info {
        if info.size.unwrap_or(0) > 0 {
            #[cfg(target_family = "unix")]
            {
                if let Some(size) = info.size {
                    out_file.set_len(size)?;
                } else {
                    return Err(anyhow!("Partition size is missing"));
                }
            }
        }
    }

    #[cfg(feature = "differential_ota")]
    let mut old_file = if args.diff {
        let old_path = args.old.join(format!("{}.img", partition_name));
        let mut file = File::open(&old_path)
            .with_context(|| format!("Failed to open original image: {:?}", old_path))?;

        // Verify old partition hash if available
        if let Some(old_partition_info) = &partition.old_partition_info {
            if let Err(e) = verify_old_partition(&mut file, old_partition_info) {
                return Err(anyhow!(
                    "Old partition verification failed for {}: {}",
                    partition_name,
                    e
                ));
            }
        }

        Some(file)
    } else {
        None
    };

    #[cfg(not(feature = "differential_ota"))]
    let mut old_file: Option<File> = None;

    for (i, op) in partition.operations.iter().enumerate() {
        process_operation(
            i,
            op,
            data_offset,
            block_size,
            payload_file,
            &mut out_file,
            old_file.as_mut().map(|f| f as &mut dyn ReadSeek),
            payload_source,
        )?;

        if let Some(pb) = &progress_bar {
            let percentage = ((i + 1) as f64 / total_ops as f64 * 100.0) as u64;
            pb.set_position(percentage);
        }
    }

    if let Some(pb) = progress_bar {
        pb.finish_with_message(format!(
            "✓ Completed {} ({} ops)",
            partition_name, total_ops
        ));
    }
    Ok(())
}

pub fn create_payload_reader(path: &PathBuf) -> Result<Box<dyn ReadSeek>> {
    let file = File::open(path)?;

    let file_size = file.metadata()?.len();

    if file_size > 10 * 1024 * 1024 {
        match unsafe { memmap2::Mmap::map(&file) } {
            Ok(mmap) => {
                struct MmapReader {
                    mmap: memmap2::Mmap,
                    position: u64,
                }

                impl Read for MmapReader {
                    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                        let start = self.position as usize;
                        if start >= self.mmap.len() {
                            return Ok(0); // EOF
                        }

                        let end = std::cmp::min(start + buf.len(), self.mmap.len());
                        let bytes_to_read = end - start;

                        buf[..bytes_to_read].copy_from_slice(&self.mmap[start..end]);
                        self.position += bytes_to_read as u64;

                        Ok(bytes_to_read)
                    }
                }

                impl Seek for MmapReader {
                    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
                        let new_pos = match pos {
                            SeekFrom::Start(offset) => offset,
                            SeekFrom::Current(offset) => {
                                if offset >= 0 {
                                    self.position.saturating_add(offset as u64)
                                } else {
                                    self.position.saturating_sub(offset.abs() as u64)
                                }
                            }
                            SeekFrom::End(offset) => {
                                let file_size = self.mmap.len() as u64;
                                if offset >= 0 {
                                    file_size.saturating_add(offset as u64)
                                } else {
                                    file_size.saturating_sub(offset.abs() as u64)
                                }
                            }
                        };

                        if new_pos > self.mmap.len() as u64 {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "Attempted to seek past end of file",
                            ));
                        }

                        self.position = new_pos;
                        Ok(self.position)
                    }
                }

                return Ok(Box::new(MmapReader { mmap, position: 0 }) as Box<dyn ReadSeek>);
            }
            Err(_) => Ok(Box::new(file) as Box<dyn ReadSeek>),
        }
    } else {
        Ok(Box::new(file) as Box<dyn ReadSeek>)
    }
}
