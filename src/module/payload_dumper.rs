use anyhow::{Context, Result, anyhow};
use bzip2::read::BzDecoder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use lzma::LzmaReader;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::Duration;

use crate::InstallOperation;
pub use crate::PartitionUpdate;
use crate::ReadSeek;
use crate::install_operation;
#[cfg(feature = "differential_ota")]
use crate::module::patch::bspatch;
use crate::module::structs::Args;
use crate::module::utils::verify_hash;

pub fn process_operation(
    operation_index: usize,
    op: &InstallOperation,
    data_offset: u64,
    block_size: u64,
    payload_file: &mut (impl Read + Seek),
    out_file: &mut (impl Write + Seek),
    #[cfg(feature = "differential_ota")]
    old_file: Option<&mut dyn ReadSeek>,
    #[cfg(not(feature = "differential_ota"))]
    _old_file: Option<&mut dyn ReadSeek>,
) -> Result<()> {
    payload_file.seek(SeekFrom::Start(data_offset + op.data_offset.unwrap_or(0)))?;
    let mut data = vec![0u8; op.data_length.unwrap_or(0) as usize];
    payload_file.read_exact(&mut data)?;

    if let Some(expected_hash) = op.data_sha256_hash.as_deref() {
        if !verify_hash(&data, expected_hash) {
            println!(
                "  Warning: Operation {} data hash mismatch.",
                operation_index
            );
            return Ok(());
        }
    }
    match op.r#type() {
        install_operation::Type::ReplaceXz => {
            let mut decompressed = Vec::new();
            match LzmaReader::new_decompressor(Cursor::new(&data)) {
                Ok(mut decompressor) => {
                    if let Err(e) = decompressor.read_to_end(&mut decompressed) {
                        println!(
                            "  Warning: Failed to decompress XZ in operation {}.  : {}",
                            operation_index, e
                        );
                        return Ok(());
                    }
                    out_file.seek(SeekFrom::Start(
                        op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                    ))?;
                    out_file.write_all(&decompressed)?;
                }
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to XZ decompression error.  : {}",
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
                    } else {
                        println!(
                            "  Warning: Skipping extent in operation {} due to insufficient decompressed data.",
                            operation_index
                        );
                        break;
                    }
                }
            }
            Err(e) => {
                println!(
                    "  Warning: Skipping operation {} due to unknown Zstd format: {}",
                    operation_index, e
                );
                return Ok(());
            }
        },
        install_operation::Type::ReplaceBz => {
            let mut decoder = BzDecoder::new(Cursor::new(&data));
            let mut decompressed = Vec::new();
            match decoder.read_to_end(&mut decompressed) {
                Ok(_) => {
                    out_file.seek(SeekFrom::Start(
                        op.dst_extents[0].start_block.unwrap_or(0) * block_size,
                    ))?;
                    out_file.write_all(&decompressed)?;
                }
                Err(e) => {
                    println!(
                        " Warning: Skipping operation {} due to unknown BZ2 format.  : {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Replace => {
            out_file.seek(SeekFrom::Start(
                op.dst_extents[0].start_block.unwrap_or(0) * block_size,
            ))?;
            out_file.write_all(&data)?;
        }
        #[cfg(feature = "differential_ota")]
        install_operation::Type::SourceCopy => {
            let old_file = old_file
                .ok_or_else(|| anyhow!("SOURCE_COPY supported only for differential OTA"))?;
            out_file.seek(SeekFrom::Start(
                op.dst_extents[0].start_block.unwrap_or(0) * block_size,
            ))?;
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                out_file.write_all(&buffer)?;
            }
        }
        #[cfg(feature = "differential_ota")]
        install_operation::Type::SourceBsdiff | install_operation::Type::BrotliBsdiff => {
            let old_file =
                old_file.ok_or_else(|| anyhow!("BSDIFF supported only for differential OTA"))?;

            let mut old_data = Vec::new();
            for ext in &op.src_extents {
                old_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                let mut buffer = vec![0u8; (ext.num_blocks.unwrap_or(0) * block_size) as usize];
                old_file.read_exact(&mut buffer)?;
                old_data.extend_from_slice(&buffer);
            }
            let new_data = match bspatch(&old_data, &data) {
                Ok(new_data) => new_data,
                Err(e) => {
                    println!(
                        "  Warning: Skipping operation {} due to failed BSDIFF patch.  : {}",
                        operation_index, e
                    );
                    return Ok(());
                }
            };
            let mut pos = 0;
            for ext in &op.dst_extents {
                let ext_size = (ext.num_blocks.unwrap_or(0) * block_size) as usize;
                let end_pos = pos + ext_size;
                if end_pos <= new_data.len() {
                    out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                    out_file.write_all(&new_data[pos..end_pos])?;
                    pos = end_pos;
                } else {
                    println!(
                        "  Warning: Skipping operation {} due to insufficient patched data.  .",
                        operation_index
                    );
                    return Ok(());
                }
            }
        }
        install_operation::Type::Zero => {
            let zeros = vec![0u8; block_size as usize];
            for ext in &op.dst_extents {
                out_file.seek(SeekFrom::Start(ext.start_block.unwrap_or(0) * block_size))?;
                for _ in 0..ext.num_blocks.unwrap_or(0) {
                    out_file.write_all(&zeros)?;
                }
            }
        }
        #[cfg(not(feature = "differential_ota"))]
        install_operation::Type::SourceCopy | 
        install_operation::Type::SourceBsdiff | 
        install_operation::Type::BrotliBsdiff => {
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

pub fn dump_partition(
    partition: &PartitionUpdate,
    data_offset: u64,
    block_size: u64,
    args: &Args,
    payload_file: &mut (impl Read + Seek),
    multi_progress: Option<&MultiProgress>,
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
        pb.set_message(format!("Processing {} ({} ops)", partition_name, total_ops));
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
        Some(
            File::open(&old_path)
                .with_context(|| format!("Failed to open original image: {:?}", old_path))?,
        )
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
    drop(out_file);
    let mut out_file = File::open(&out_path)
        .with_context(|| format!("Failed to reopen {} for hash verification", partition_name))?;
    if let Some(info) = &partition.new_partition_info {
        if info.hash.as_ref().map_or(true, |hash| hash.is_empty()) {
            let hash_pb = if let Some(mp) = multi_progress {
                let pb = mp.add(ProgressBar::new_spinner());
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner:.green} {msg}")
                        .unwrap(),
                );
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message(format!("Verifying hash for {}", partition_name));
                Some(pb)
            } else {
                None
            };
            out_file.seek(SeekFrom::Start(0))?;
            let mut hasher = Sha256::new();
            io::copy(&mut out_file, &mut hasher)?;
            let hash = hasher.finalize();
            if let Some(pb) = hash_pb {
                if hash.as_slice() != info.hash.as_deref().unwrap_or(&[]) {
                    pb.finish_with_message(format!("✕ Hash mismatch for {}", partition_name));
                } else {
                    pb.finish_with_message(format!("✓ Hash verified for {}", partition_name));
                }
            }
        }
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
