# payload-dumper-rust
Android OTA payload dumper written in Rust.

### What is Payload?

-  Android payload is a file that contains ROM partitions like boot , system, vendor . and others. Payload Dumper extracts these partitions from the payload.bin file

## features
- Extracts all or individual images directly from payload.bin or ROM ZIP files with minimal time difference.

- Supports extracting individual partitions from URLs without downloading the full ROM ZIP.

- All decompression processes run in parallel for improved performance. ( can be customised by using`--no-parallel` or `--threads <n>` as argument )

---
- Output partitions Verification ✅
- Parallel Extraction of direct zip/payload files ✅
- Selective Partition Extraction ✅
- Direct Extraction from URL ✅
---

## How To Use 
- Download Binaries for your respective Platform from [releases section](https://github.com/rhythmcache/payload-dumper-rust/releases/tag/0.2.0)
- If you are using a rooted android device you might want to install it as a [magisk module](https://github.com/rhythmcache/payload-dumper-rust/releases/download/0.2.0/payload_dumper-android-magisk-module.zip)

- For unrooted Android - run this in termux to install it
  ```
  bash <(curl -L "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/termux-install.sh")
  ```
  



Here are some performance metrics from a **Poco X4 Pro (SD695, 8GB RAM)** running in Termux:


- payload/zip file used :- [comet-ota-ad1a.240530.030-98066022.zip](https://dl.google.com/dl/android/aosp/comet-ota-ad1a.240530.030-98066022.zip) (2.53GB)

- **Direct Payload Extraction**: Extracting partitions directly from the `payload.bin` took **2 minutes 26 seconds**.

- **ZIP File Extraction**: Extracting partitions directly from the ZIP file took **2 minutes 30 seconds**, just **4 seconds difference**

- It can also extract partition directly from **URL** without having you to download the full rom zip file



### Screenshots
- **Direct Payload Extraction**:  
  <img src="./Screenshot_20250304-175923_Termux.png" width="75%">

- **ZIP File Extraction**:  
  <img src="./Screenshot_20250304-175502_Termux.png" width="75%">

- **Remote URL Extraction**:  
  <img src="./Screenshot_20250304-180030_Termux.png" width="75%">

### Usage :
#### Basic Usage

To extract partitions from a payload file, run the following command:

```bash
payload-dumper <path/to/payload.bin> --out output_directory
```
#### Direct ZIP Processing

it can directly process payloads from ZIP files without requiring manual extraction. Simply provide the path to the ZIP file:

```bash
./payload-dumper <path/to/ota.zip> --out <output_directory>
```

#### Remote Payloads

it can also handle payloads/zips directly using url.  Simply provide the URL as path. this is very slow compared to local 
extraction though.

```bash
./payload-dumper https://example.com/payload.bin
```
#### Individual partitions extraction.

to extract individual partitions from payloads/URL/zips , use `--images` and enter the name of partitions you want to extract comma-separated.

for example to just extract `boot` and `vendor_boot` from `url/zip/payload` , simply run

```
payload_dumper --images boot,vendor_boot <https://example.com/zip>
```
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
      List available partitions in the payload.
  --metadata  
      Save payload metadata as JSON.
```

#### Dependencies :
- See [Cargo.toml](./Cargo.toml)


#### Build :
To build this , you'll need:
- Rust compiler and Cargo
- protobuf-compiler
- Other obvious dependencies
- You may also need to link against libzip, zlib, and liblzma.

  
- ***To Build On Termux , Simply Run***
```
bash <(curl -L "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/termux-build.sh")
```


### Credits
- This tool is inspired from [vm03/payload_dumper](https://github.com/vm03/payload_dumper)
- [update_metadata.proto](https://android.googlesource.com/platform/system/update_engine/+/HEAD/update_metadata.proto)
