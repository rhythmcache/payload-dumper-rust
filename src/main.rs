mod module;
use crate::module::args::Args;
#[cfg(feature = "remote_ota")]
use crate::module::http::HttpReader;
#[cfg(feature = "metadata")]
use crate::module::metadata::save_metadata;
use crate::module::payload_dumper::{PayloadSource, create_payload_reader, dump_partition};
use crate::module::utils::list_partitions;
use crate::module::utils::{format_elapsed_time, format_size, is_differential_ota};
use crate::module::verify::verify_partitions_hash;
#[cfg(feature = "local_zip")]
use crate::module::zip::local_zip::OptimizedZipPayloadReader;
#[cfg(feature = "remote_ota")]
use crate::module::zip::remote_zip::RemoteZipReader;
use anyhow::{Result, anyhow};
use byteorder::{BigEndian, ReadBytesExt};
use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
#[cfg(feature = "remote_ota")]
use lazy_static::lazy_static;
use prost::Message;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
#[cfg(feature = "remote_ota")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "remote_ota")]
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[cfg(feature = "remote_ota")]
lazy_static! {
    static ref FILE_SIZE_INFO_SHOWN: AtomicBool = AtomicBool::new(false);
}

include!("proto/update_metadata.rs");

trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

fn main() -> Result<()> {
    let args = Args::parse();

    // Validate metadata feature usage
    #[cfg(not(feature = "metadata"))]
    if args.metadata {
        return Err(anyhow!(
            "Metadata functionality requires the 'metadata' feature to be enabled. Please recompile with --features metadata"
        ));
    }

    let thread_count = if args.no_parallel {
        1
    } else if let Some(threads) = args.threads {
        threads
    } else {
        num_cpus::get()
    };

    rayon::ThreadPoolBuilder::new()
        .num_threads(thread_count)
        .build_global()?;

    let start_time = Instant::now();

    let multi_progress = MultiProgress::new();
    let main_pb = multi_progress.add(ProgressBar::new_spinner());
    main_pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    main_pb.enable_steady_tick(Duration::from_millis(100));
    let payload_path_str = args.payload_path.to_string_lossy().to_string();

    // Check if we're outputting to stdout
    let is_stdout = args.out.to_string_lossy() == "-";

    // Check if it's a URL - only available with remote_ota feature
    #[cfg(feature = "remote_ota")]
    let is_url =
        payload_path_str.starts_with("http://") || payload_path_str.starts_with("https://");
    #[cfg(not(feature = "remote_ota"))]
    let is_url = false;

    // Validate URL usage when feature is disabled
    #[cfg(not(feature = "remote_ota"))]
    if payload_path_str.starts_with("http://") || payload_path_str.starts_with("https://") {
        return Err(anyhow!(
            "Network-based payload dumping requires the 'remote_ota' feature to be enabled. Please recompile with --features remote_ota"
        ));
    }

    // Check if it's a local ZIP file
    let is_local_zip =
        !is_url && args.payload_path.extension().and_then(|e| e.to_str()) == Some("zip");

    // Validate local ZIP usage when feature is disabled
    #[cfg(not(feature = "local_zip"))]
    if is_local_zip {
        return Err(anyhow!(
            "Local ZIP file support requires the 'local_zip' feature to be enabled. Please recompile with --features local_zip"
        ));
    }

    let payload_source = if is_url {
        if payload_path_str.ends_with(".zip") {
            PayloadSource::RemoteZip
        } else {
            PayloadSource::RemoteHttp
        }
    } else if is_local_zip {
        PayloadSource::LocalZip
    } else {
        PayloadSource::LocalFile
    };

    main_pb.set_message("Opening file...");

    if !is_url {
        if let Ok(metadata) = fs::metadata(&args.payload_path) {
            if metadata.len() > 1024 * 1024 {
                if is_stdout {
                    eprintln!(
                        "Processing file: {}, size: {}",
                        payload_path_str,
                        format_size(metadata.len())
                    );
                } else {
                    println!(
                        "Processing file: {}, size: {}",
                        payload_path_str,
                        format_size(metadata.len())
                    );
                }
            }
        }
    }

    let mut payload_reader: Box<dyn ReadSeek> = if is_url {
        #[cfg(feature = "remote_ota")]
        {
            main_pb.set_message("Initializing remote connection...");
            let url = payload_path_str.clone();
            let is_zip = url.ends_with(".zip");

            let content_type = if !is_zip {
                let http_reader = HttpReader::new_silent(url.clone());
                if let Ok(reader) = &http_reader {
                    let file_size = reader.content_length;
                    main_pb.set_message("Connection established");
                    if file_size > 1024 * 1024 && !FILE_SIZE_INFO_SHOWN.swap(true, Ordering::SeqCst)
                    {
                        if is_stdout {
                            eprintln!("- Remote file size: {}", format_size(file_size));
                        } else {
                            println!("- Remote file size: {}", format_size(file_size));
                        }
                    }
                    reader.content_type.clone()
                } else {
                    None
                }
            } else {
                None
            };

            if is_zip || content_type.as_deref() == Some("application/zip") {
                let reader = RemoteZipReader::new_for_parallel_with_user_agent(
                    url,
                    args.user_agent.as_deref(),
                )?;
                let file_size = reader.http_reader.content_length;
                main_pb.set_message("Connection established");
                if file_size > 1024 * 1024 && !FILE_SIZE_INFO_SHOWN.swap(true, Ordering::SeqCst) {
                    if is_stdout {
                        eprintln!("- Remote ZIP size: {}", format_size(file_size));
                    } else {
                        println!("- Remote ZIP size: {}", format_size(file_size));
                    }
                }
                Box::new(reader) as Box<dyn ReadSeek>
            } else {
                let reader =
                    HttpReader::new_with_user_agent(url, args.user_agent.as_deref(), true)?;
                let file_size = reader.content_length;
                main_pb.set_message("Connection established");
                if file_size > 1024 * 1024 && !FILE_SIZE_INFO_SHOWN.swap(true, Ordering::SeqCst) {
                    if is_stdout {
                        eprintln!("- Remote file size: {}", format_size(file_size));
                    } else {
                        println!("- Remote file size: {}", format_size(file_size));
                    }
                }
                Box::new(reader) as Box<dyn ReadSeek>
            }
        }
        #[cfg(not(feature = "remote_ota"))]
        {
            // This branch should never be reached due to earlier validation
            return Err(anyhow!(
                "Internal error: URL processing attempted without remote_ota feature"
            ));
        }
    } else if is_local_zip {
        #[cfg(feature = "local_zip")]
        {
            OptimizedZipPayloadReader::from_file(&args.payload_path)
                .map_err(|e| anyhow::anyhow!("Failed to open ZIP file: {}", e))
                .map(|reader| Box::new(reader) as Box<dyn ReadSeek>)?
        }
        #[cfg(not(feature = "local_zip"))]
        {
            // This branch should never be reached due to earlier validation
            return Err(anyhow!(
                "Internal error: Local ZIP processing attempted without local_zip feature"
            ));
        }
    } else {
        Box::new(File::open(&args.payload_path)?) as Box<dyn ReadSeek>
    };

    if args.out.to_string_lossy() != "-" {
        fs::create_dir_all(&args.out)?;
    }

    let mut magic = [0u8; 4];
    payload_reader.read_exact(&mut magic)?;
    if magic != *b"CrAU" {
        return Err(anyhow!("Invalid payload file: magic 'CrAU' not found"));
    }
    let file_format_version = payload_reader.read_u64::<BigEndian>()?;
    if file_format_version != 2 {
        return Err(anyhow!(
            "Unsupported payload version: {}",
            file_format_version
        ));
    }
    let manifest_size = payload_reader.read_u64::<BigEndian>()?;
    let metadata_signature_size = payload_reader.read_u32::<BigEndian>()?;
    main_pb.set_message("Reading manifest...");
    let mut manifest = vec![0u8; manifest_size as usize];
    payload_reader.read_exact(&mut manifest)?;
    let mut metadata_signature = vec![0u8; metadata_signature_size as usize];
    payload_reader.read_exact(&mut metadata_signature)?;
    let data_offset = payload_reader.stream_position()?;
    let manifest = DeltaArchiveManifest::decode(&manifest[..])?;

    if is_differential_ota(&manifest) {
        #[cfg(feature = "differential_ota")]
        {
            if !args.diff {
                return Err(anyhow!(
                    "This appears to be a differential OTA package. Use --diff argument and provide the original images directory with --old <path>"
                ));
            }
        }

        #[cfg(not(feature = "differential_ota"))]
        {
            return Err(anyhow!(
                "Differential OTA is not supported in this build. Recompile with --features differential_ota"
            ));
        }
    }

    if let Some(security_patch) = &manifest.security_patch_level {
        if is_stdout {
            eprintln!("- Security Patch: {}", security_patch);
        } else {
            println!("- Security Patch: {}", security_patch);
        }
    }

    #[cfg(feature = "metadata")]
    if args.metadata && !args.list {
        main_pb.set_message("Extracting metadata...");

        match save_metadata(&manifest, &args.out, data_offset) {
            Ok(json) => {
                if is_stdout {
                    println!("{}", json);
                } else {
                    println!(
                        "✓ Metadata saved to: {}/payload_metadata.json",
                        args.out.display()
                    );
                }
                multi_progress.clear()?;
                return Ok(());
            }
            Err(e) => {
                main_pb.finish_with_message("Failed to save metadata");
                return Err(e);
            }
        }
    }

    if args.list {
        main_pb.finish_and_clear();
        multi_progress.clear()?;

        #[cfg(feature = "metadata")]
        if args.metadata {
            match save_metadata(&manifest, &args.out, data_offset) {
                Ok(json) => {
                    if is_stdout {
                        println!("{}", json);
                        return Ok(());
                    } else {
                        println!(
                            "✓ Metadata saved to: {}/payload_metadata.json",
                            args.out.display()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Failed to save metadata: {}", e);
                }
            }
        }

        println!();
        payload_reader.seek(SeekFrom::Start(0))?;
        return list_partitions(&mut payload_reader);
    }

    let block_size = manifest.block_size.unwrap_or(4096);
    let partitions_to_extract: Vec<_> = if args.images.is_empty() {
        manifest.partitions.iter().collect()
    } else {
        let images = args.images.split(',').collect::<HashSet<_>>();
        manifest
            .partitions
            .iter()
            .filter(|p| images.contains(p.partition_name.as_str()))
            .collect()
    };
    if partitions_to_extract.is_empty() {
        main_pb.finish_with_message("No partitions to extract");
        multi_progress.clear()?;
        return Ok(());
    }
    main_pb.set_message(format!(
        "Found {} partitions to extract",
        partitions_to_extract.len()
    ));

    let use_parallel = ((!is_url
        && (is_local_zip
            || args.payload_path.extension().and_then(|e| e.to_str()) == Some("bin")))
        || is_url)
        && !args.no_parallel;
    main_pb.set_message(if use_parallel {
        "Extracting Partitions..."
    } else {
        "Processing partitions..."
    });
    let multi_progress = Arc::new(multi_progress);
    let args = Arc::new(args);

    let mut failed_partitions = Vec::new();

    if use_parallel {
        #[cfg(feature = "local_zip")]
        let payload_path = Arc::new(args.payload_path.to_str().unwrap_or_default().to_string());
        #[cfg(feature = "remote_ota")]
        let payload_url = Arc::new(if is_url {
            payload_path_str.clone()
        } else {
            String::new()
        });

        let max_retries = 3;
        let num_cpus = num_cpus::get();
        let chunk_size = std::cmp::max(1, partitions_to_extract.len() / num_cpus);

        let active_readers = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let max_concurrent_readers = num_cpus;

        let results: Vec<_> = partitions_to_extract
            .par_chunks(chunk_size)
            .flat_map(|chunk| {
                chunk.par_iter().map(|partition| {
                    let active_readers = Arc::clone(&active_readers);
                    let partition_name = partition.partition_name.clone();

                    let result = (0..max_retries)
                        .find_map(|attempt| {
                            if attempt > 0 {
                                let delay = 100 * (1 << attempt.min(4));
                                std::thread::sleep(Duration::from_millis(delay));
                            }

                            let needs_reader_limit = !is_url && is_local_zip;

                            if needs_reader_limit {
                                let current =
                                    active_readers.load(std::sync::atomic::Ordering::SeqCst);
                                if current >= max_concurrent_readers {
                                    while active_readers.load(std::sync::atomic::Ordering::SeqCst)
                                        >= max_concurrent_readers
                                    {
                                        std::thread::sleep(Duration::from_millis(10));
                                    }
                                }

                                active_readers.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            }

                            let reader_result = if is_url {
                                #[cfg(feature = "remote_ota")]
                                {
                                    RemoteZipReader::new_for_parallel_with_user_agent(
                                        (*payload_url).clone(),
                                        args.user_agent.as_deref(),
                                    )
                                    .map(|reader| Box::new(reader) as Box<dyn ReadSeek>)
                                }
                                #[cfg(not(feature = "remote_ota"))]
                                {
                                    Err(anyhow!("Remote OTA feature not enabled"))
                                }
                            } else if is_local_zip {
                                #[cfg(feature = "local_zip")]
                                {
                                    let result = OptimizedZipPayloadReader::from_file(
                                        (*payload_path).clone(),
                                    )
                                    .map(|reader| Box::new(reader) as Box<dyn ReadSeek>)
                                    .map_err(|e| {
                                        anyhow::anyhow!("Failed to create ZIP reader: {}", e)
                                    }); // Convert io::Error to anyhow::Error
                                    active_readers
                                        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                    result
                                }
                                #[cfg(not(feature = "local_zip"))]
                                {
                                    Err(anyhow!("Local ZIP feature not enabled"))
                                }
                            } else {
                                create_payload_reader(&args.payload_path).map_err(|e| {
                                    anyhow::anyhow!("Failed to create payload reader: {}", e)
                                })
                            };

                            let mut reader = match reader_result {
                                Ok(reader) => reader,
                                Err(e) => {
                                    return if attempt == max_retries - 1 {
                                        Some(Err((partition_name.clone(), e)))
                                    } else {
                                        None // Try again
                                    };
                                }
                            };

                            match dump_partition(
                                partition,
                                data_offset,
                                block_size as u64,
                                &args,
                                &mut reader,
                                Some(&multi_progress),
                                payload_source,
                            ) {
                                Ok(()) => Some(Ok(())),
                                Err(e) => {
                                    if attempt == max_retries - 1 {
                                        Some(Err((partition_name.clone(), e)))
                                    } else {
                                        None // Try again
                                    }
                                }
                            }
                        })
                        .unwrap_or_else(|| {
                            Err((partition_name, anyhow!("All retry attempts failed")))
                        });

                    result
                })
            })
            .collect();
        for result in results {
            if let Err((partition_name, error)) = result {
                eprintln!("Failed to process partition {}: {}", partition_name, error);
                failed_partitions.push(partition_name);
            }
        }
        if !failed_partitions.is_empty() {
            main_pb.set_message(format!(
                "Retrying {} failed partitions sequentially...",
                failed_partitions.len()
            ));

            let mut reader: Box<dyn ReadSeek> = if is_url {
                #[cfg(feature = "remote_ota")]
                {
                    Box::new(RemoteZipReader::new_for_parallel(payload_url.to_string())?)
                        as Box<dyn ReadSeek>
                }
                #[cfg(not(feature = "remote_ota"))]
                {
                    return Err(anyhow!("Remote OTA feature not enabled"));
                }
            } else {
                payload_reader
            };

            let mut remaining_failed_partitions = Vec::new();
            for partition in partitions_to_extract
                .iter()
                .filter(|p| failed_partitions.contains(&p.partition_name))
            {
                if let Err(e) = dump_partition(
                    partition,
                    data_offset,
                    block_size as u64,
                    &args,
                    &mut reader,
                    Some(&multi_progress),
                    payload_source,
                ) {
                    eprintln!(
                        "Failed to process partition {} in sequential mode: {}",
                        partition.partition_name, e
                    );
                    remaining_failed_partitions.push(partition.partition_name.clone());
                }
            }
            failed_partitions = remaining_failed_partitions;
        }
    } else {
        for partition in &partitions_to_extract {
            if let Err(e) = dump_partition(
                partition,
                data_offset,
                block_size as u64,
                &args,
                &mut payload_reader,
                Some(&multi_progress),
                payload_source,
            ) {
                eprintln!(
                    "Failed to process partition {}: {}",
                    partition.partition_name, e
                );
                failed_partitions.push(partition.partition_name.clone());
            }
        }
    }

    if !args.no_verify {
        main_pb.set_message("Verifying partition hashes...");

        let partitions_to_verify: Vec<&PartitionUpdate> = partitions_to_extract
            .iter()
            .filter(|p| !failed_partitions.contains(&p.partition_name))
            .copied()
            .collect();

        match verify_partitions_hash(&partitions_to_verify, &args, &multi_progress) {
            Ok(failed_verifications) => {
                if !failed_verifications.is_empty() {
                    eprintln!(
                        "Hash verification failed for {} partitions.",
                        failed_verifications.len()
                    );
                }
            }
            Err(e) => {
                eprintln!("Error during hash verification: {}", e);
            }
        }
    } else {
        main_pb.set_message("Hash verification skipped (--no-verify flag)");
    }

    let elapsed_time = format_elapsed_time(start_time.elapsed());

    if failed_partitions.is_empty() {
        main_pb.finish_with_message(format!(
            "All partitions extracted successfully! (in {})",
            elapsed_time
        ));
        println!(
            "\nExtraction completed successfully in {}. Output directory: {:?}",
            elapsed_time, args.out
        );
    } else {
        main_pb.finish_with_message(format!(
            "Completed with {} failed partitions. (in {})",
            failed_partitions.len(),
            elapsed_time
        ));
        println!(
            "\nExtraction completed with {} failed partitions in {}. Output directory: {:?}",
            failed_partitions.len(),
            elapsed_time,
            args.out
        );
    }

    Ok(())
}
