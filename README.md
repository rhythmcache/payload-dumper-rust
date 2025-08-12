# 🚀 payload-dumper-rust

Android OTA payload dumper written in Rust.

## 📖 What is Payload?

Android payload is a file that contains ROM partitions like boot, system, vendor and others. Payload Dumper extracts these partitions from the payload.bin file.

## ✨ Features

- Extracts all or individual images directly from payload.bin or ROM ZIP file
- Supports extracting individual partitions from URLs without downloading the full ROM ZIP
- All decompression processes run in parallel for improved performance (can be customised by using `--no-parallel` or `--threads <n>` as argument)

---

✅ Output partitions Verification  
✅ Parallel Extraction  
✅ Selective Partition Extraction  
✅ Direct Extraction from URL  

---

## 📥 How To Use

- Download Binaries for your respective Platform from [releases section](https://github.com/rhythmcache/payload-dumper-rust/releases)
- If you are using a rooted android device you might want to install it as a [magisk module](https://github.com/rhythmcache/payload-dumper-rust/releases/download/0.3.0/payload_dumper-android-magisk-module.zip)

- or Run this in termux / Linux Terminal to install:
  ```bash
  bash <(curl -sSL "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/scripts/install.sh")
  ```

- To install on windows, run this in Powershell:
  ```powershell
  powershell -NoExit -ExecutionPolicy Bypass -Command "Invoke-RestMethod -Uri 'https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/scripts/install.ps1' | Invoke-Expression"
  ```

### Install via Cargo

If you have Rust and Cargo installed, you can install this tool with:

```bash
cargo install payload_dumper
```

---

## ⚡ Performance Metrics

Here are the performance metrics for **Payload Dumper Rust** running on a **Poco X4 Pro (SD695, 8GB RAM)** in Termux. The test file used is [comet-ota-ad1a.240530.030-98066022.zip](https://dl.google.com/dl/android/aosp/comet-ota-ad1a.240530.030-98066022.zip) (2.53GB).

| **Extraction Method** | **Time Taken** | **Notes** |
|-----------------------|----------------|-----------|
| **Direct Payload Extraction** | **2 minutes 26 seconds** | Extracting directly from `payload.bin` |
| **ZIP File Extraction** | **2 minutes 30 seconds** | Extracting directly from the ZIP file |
| **Remote URL Extraction** | **Slower** | Depends on network speed |

---

## 📸 Screenshots

- **Direct Payload Extraction**:  
  ![Direct Payload Extraction](https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/photos/Screenshot_20250304-175923_Termux.png)

- **ZIP File Extraction**:  
  ![ZIP File Extraction](https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/photos/Screenshot_20250304-175502_Termux.png)

- **Remote URL Extraction**:  
  ![Remote URL Extraction](https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/photos/Screenshot_20250304-180030_Termux.png)

---

## 🛠️ Usage

### Basic Usage

To extract partitions from a payload file, run the following command:

```bash
payload_dumper <path/to/payload.bin> --out output_directory
```

### Direct ZIP Processing

It can directly process payloads from ZIP files without requiring manual extraction. Simply provide the path to the ZIP file:

```bash
./payload_dumper <path/to/ota.zip> --out <output_directory>
```

### Remote Payloads

It can also handle payloads/zips directly using url. Simply provide the URL as path. This is very slow compared to local extraction though.

```bash
./payload_dumper https://example.com/payload.bin
```

### Individual partitions extraction

To extract individual partitions from payloads/URL/zips, use `--images` and enter the name of partitions you want to extract comma-separated.

For example to just extract `boot` and `vendor_boot` from `url/zip/payload`, simply run:

```bash
payload_dumper --images boot,vendor_boot <https://example.com/zip>
```

---

```
Usage: payload_dumper [OPTIONS] <PAYLOAD_PATH>

Arguments:
  <PAYLOAD_PATH>  
      Path to the payload file.
  --out <OUT>  
      Output directory for extracted partitions. [default: output]
  --diff  
      Enable differential OTA mode (requires --old).
  --old <OLD>  
      Path to the directory containing old partition images (required for --diff). [default: old]
  --images <IMAGES>  
      Comma-separated list of partition names to extract (default: all partitions)
  --threads <THREADS>  
      Number of threads to use for parallel processing.
  --list  
      List available partitions
  --metadata
      Save complete metadata as json ( use -o - to write to stdout )
  --user-agent
      Custom User-Agent string for HTTP requests (only used with remote URLs)
  --no-verify
      Skip Hash Verification    
  --no-parallel
      Disable parallel Extraction
```

---

## 🔧 Dependencies

- [Cargo.toml](./Cargo.toml)
- [update_metadata.proto](https://android.googlesource.com/platform/system/update_engine/+/HEAD/update_metadata.proto)

---

## 🏗️ Build

To build this, you'll need:
- Rust compiler and Cargo

---

## 🙏 Credits

This tool is inspired from [vm03/payload_dumper](https://github.com/vm03/payload_dumper)
