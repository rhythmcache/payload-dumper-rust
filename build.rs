// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust
//
// This file is part of payload-dumper-rust. It implements components used for
// extracting and processing Android OTA payloads.

use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    let build_time = chrono::Utc::now()
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string();
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", build_time);

    // fallback to "unknown" if not available
    let git_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_COMMIT={}", git_hash);

    let git_hash_short = if git_hash != "unknown" && git_hash.len() >= 8 {
        git_hash[..8].to_string()
    } else {
        git_hash.clone()
    };
    println!("cargo:rustc-env=GIT_COMMIT_SHORT={}", git_hash_short);

    let git_branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_BRANCH={}", git_branch);

    let mut features = Vec::new();

    if env::var("CARGO_FEATURE_REMOTE_ZIP").is_ok() {
        features.push("remote_zip");
    }
    if env::var("CARGO_FEATURE_LOCAL_ZIP").is_ok() {
        features.push("local_zip");
    }
    if env::var("CARGO_FEATURE_METADATA").is_ok() {
        features.push("metadata");
    }
    if env::var("CARGO_FEATURE_HICKORY_DNS").is_ok() {
        features.push("hickory_dns");
    }
    if env::var("CARGO_FEATURE_PREFETCH").is_ok() {
        features.push("prefetch");
    }
    let features_str = if features.is_empty() {
        "none".to_string()
    } else {
        features.join(",")
    };
    println!("cargo:rustc-env=BUILD_FEATURES={}", features_str);

    println!(
        "cargo:rustc-env=BUILD_TARGET={}",
        env::var("TARGET").unwrap()
    );

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET_ARCH={}", target_arch);

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET_OS={}", target_os);

    println!(
        "cargo:rustc-env=BUILD_PROFILE={}",
        env::var("PROFILE").unwrap()
    );

    let opt_level = env::var("OPT_LEVEL").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=OPT_LEVEL={}", opt_level);

    let rustc_version = Command::new("rustc")
        .args(["--version"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=RUSTC_VERSION={}", rustc_version);

    let build_host = env::var("HOST").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_HOST={}", build_host);

    // set platform specific default user agent
    let default_user_agent = match target_os.as_str() {
        "windows" => {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        }
        "macos" => {
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        }
        "linux" => {
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        }
        "android" => {
            "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36"
        }
        "ios" => {
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
        }
        _ => {
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        } // fallback to Linux
    };
    println!("cargo:rustc-env=DEFAULT_USER_AGENT={}", default_user_agent);

    if std::path::Path::new(".git/HEAD").exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
    }

    // Protobuf compilation
    compile_protos();
}

fn compile_protos() {
    let proto_file = "proto/update_metadata.proto";
    let proto_path = Path::new(proto_file);
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_file = Path::new(&out_dir).join("chromeos_update_engine.rs");
    let precompiled = Path::new("src/chromeos_update_engine.rs");

    // Check if proto file exists
    if !proto_path.exists() {
        println!(
            "cargo:warning=Proto file not found at {}, using pre-compiled version",
            proto_file
        );
        copy_precompiled_to_out(&precompiled, &out_file);
        return;
    }

    // Check if protoc is available
    let protoc_available = Command::new("protoc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    if !protoc_available {
        println!("cargo:warning=protoc command not found, using pre-compiled version");
        copy_precompiled_to_out(&precompiled, &out_file);
        return;
    }

    // Both proto file and protoc exist, proceed with compilation
    println!("cargo:rerun-if-changed={}", proto_file);

    match prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(&[proto_file], &["proto/"])
    {
        Ok(_) => {
            println!(
                "cargo:warning=Successfully compiled protobuf to {}/chromeos_update_engine.rs",
                out_dir
            );
        }
        Err(e) => {
            println!(
                "cargo:warning=Failed to compile protobuf: {}. Using pre-compiled version.",
                e
            );
            copy_precompiled_to_out(&precompiled, &out_file);
        }
    }
}

fn copy_precompiled_to_out(precompiled: &Path, out_file: &Path) {
    if precompiled.exists() {
        if let Err(e) = std::fs::copy(precompiled, out_file) {
            panic!("Failed to copy pre-compiled protobuf file: {}", e);
        } else {
            println!("cargo:warning=Copied pre-compiled protobuf to OUT_DIR");
        }
    } else {
        panic!("Neither protoc nor pre-compiled protobuf file found!");
    }
}
