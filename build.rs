#[cfg(windows)]
fn link(use_static: bool) {
    let mut config = vcpkg::Config::new();
    if use_static {
        config.r#static(true);
        println!("cargo:rustc-link-lib=static=zip");
        println!("cargo:rustc-link-lib=static=z");
    } else {
        println!("cargo:rustc-link-lib=zip");
        println!("cargo:rustc-link-lib=z");
    }
    config
        .find_package("libzip")
        .expect("Failed to find libzip via vcpkg");
}

#[cfg(not(windows))]
fn link(use_static: bool) {
    let mut config = pkg_config::Config::new();
    if use_static {
        config.statik(true);
        println!("cargo:rustc-link-lib=static=zip");
        println!("cargo:rustc-link-lib=static=z");
    } else {
        println!("cargo:rustc-link-lib=zip");
        println!("cargo:rustc-link-lib=z");
    }
    config
        .probe("libzip")
        .expect("Failed to find libzip via pkg-config");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let use_static = std::env::var("CARGO_FEATURE_STATIC").is_ok()
        || std::env::var("LIBZIP_STATIC").is_ok();

    link(use_static);
}
