fn main() -> Result<(), Box<dyn std::error::Error>> {
   // let mut config = prost_build::Config::new();
  //  config.type_attribute(".", "#[derive(serde::Serialize)]");
 //   config.compile_protos(&["proto/update_metadata.proto"], &["proto/"])?;
    
    let target = std::env::var("TARGET").unwrap_or_default();
    let is_android = target.contains("android");
    let is_musl = target.contains("musl");
    let is_windows = target.contains("windows");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let use_static_libs = std::env::var("STATIC_LIBS").is_ok();
    
    if use_static_libs {
        println!("cargo:warning=Using static linking for libraries");
    }
    
    if is_android {
        if target.contains("aarch64") {
            println!(
                "cargo:rustc-link-search=native={}/lib/android/arm64-v8a",
                manifest_dir
            );
            println!("cargo:warning=Building for Android aarch64 architecture");
        } else if target.contains("armv7") {
            println!(
                "cargo:rustc-link-search=native={}/lib/android/armv7",
                manifest_dir
            );
            println!("cargo:warning=Building for Android armv7 architecture");
        } else if target.contains("x86_64") {
            println!(
                "cargo:rustc-link-search=native={}/lib/android/x86_64",
                manifest_dir
            );
            println!("cargo:warning=Building for Android x86_64 architecture");
        } else if target.contains("i686") || target.contains("x86") {
            println!(
                "cargo:rustc-link-search=native={}/lib/android/x86",
                manifest_dir
            );
            println!("cargo:warning=Building for Android x86 architecture");
        } else {
            println!(
                "cargo:warning=Building for unknown Android architecture: {}",
                target
            );
        }
        println!("cargo:warning=Target architecture: {}", target);
        println!("cargo:rustc-link-arg=-fuse-ld=lld");
    } else if is_musl {
        if target.contains("x86_64") {
            println!(
                "cargo:rustc-link-search=native={}/lib/musl/x86_64",
                manifest_dir
            );
            println!("cargo:warning=Building for musl x86_64 architecture");
        } else if target.contains("aarch64") {
            println!(
                "cargo:rustc-link-search=native={}/lib/musl/aarch64",
                manifest_dir
            );
            println!("cargo:warning=Building for musl aarch64 architecture");
        } else if target.contains("arm") {
            println!(
                "cargo:rustc-link-search=native={}/lib/musl/armv7",
                manifest_dir
            );
            println!("cargo:warning=Building for musl arm architecture");
        } else if target.contains("i686") {
            println!(
                "cargo:rustc-link-search=native={}/lib/musl/x86",
                manifest_dir
            );
            println!("cargo:warning=Building for musl i686 architecture");
        } else if target.contains("riscv64") {
            println!(
                "cargo:rustc-link-search=native={}/lib/musl/riscv64",
                manifest_dir
            );
            println!("cargo:warning=Building for musl riscv64 architecture");
        } else {
            println!(
                "cargo:warning=Building for unknown musl architecture: {}",
                target
            );
            println!("cargo:rustc-link-search=native={}/lib", manifest_dir);
        }
    } else if is_windows {
        if target.contains("x86_64") {
            println!(
                "cargo:rustc-link-search=native={}/lib/win/x86_64",
                manifest_dir
            );
            println!("cargo:warning=Building for Windows x86_64 architecture");
        } else if target.contains("i686") || target.contains("x86") {
            println!(
                "cargo:rustc-link-search=native={}/lib/win/x86",
                manifest_dir
            );
            println!("cargo:warning=Building for Windows x86 architecture");
        } else if target.contains("aarch64") {
            println!(
                "cargo:rustc-link-search=native={}/lib/win/aarch64",
                manifest_dir
            );
            println!("cargo:warning=Building for Windows ARM64 architecture");
        }
    }
    
    if use_static_libs {
        println!("cargo:rustc-link-lib=static=lzma");
        println!("cargo:rustc-link-lib=static=zip");
        println!("cargo:rustc-link-lib=static=z");
    } else {
        println!("cargo:rustc-link-lib=lzma");
        println!("cargo:rustc-link-lib=zip");
        println!("cargo:rustc-link-lib=z");
    }
    
    Ok(())
}
