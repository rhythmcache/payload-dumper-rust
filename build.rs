use std::env;
use std::process::Command;

fn main() {
    let build_time = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
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
    
    if env::var("CARGO_FEATURE_REMOTE_OTA").is_ok() {
        features.push("remote_ota");
    }
    if env::var("CARGO_FEATURE_LOCAL_ZIP").is_ok() {
        features.push("local_zip");
    }
    if env::var("CARGO_FEATURE_METADATA").is_ok() {
        features.push("metadata");
    }
    if env::var("CARGO_FEATURE_DIFFERENTIAL_OTA").is_ok() {
        features.push("differential_ota");
    }
    if env::var("CARGO_FEATURE_HICKORY_DNS").is_ok() {
        features.push("hickory-dns");
    }
    
    let features_str = if features.is_empty() {
        "none".to_string()
    } else {
        features.join(",")
    };
    println!("cargo:rustc-env=BUILD_FEATURES={}", features_str);

    
    println!("cargo:rustc-env=BUILD_TARGET={}", env::var("TARGET").unwrap());

    
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET_ARCH={}", target_arch);

    
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET_OS={}", target_os);

    
    println!("cargo:rustc-env=BUILD_PROFILE={}", env::var("PROFILE").unwrap());

    
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

    
    if std::path::Path::new(".git/HEAD").exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
    }
}
