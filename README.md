# payload-dumper-rust 🦀

A fast and efficient Android OTA payload dumper written in Rust.

## What is this?

If you've ever downloaded an Android OTA update and wondered how to extract the actual system images from it, this tool is for you. Android OTA packages contain a `payload.bin` file that holds all the partition images (boot, system, vendor, etc.). This tool extracts those partitions so you can work with them directly.

## Why use this?

- **Fast**: All decompression runs in parallel
- **Flexible**: Extract from payload.bin files, ROM ZIPs, or even direct URLs
- **Smart**: Only download what you need when extracting from URLs
- **Reliable**: Verifies extracted partitions to ensure integrity
- **Cross-platform**: Works on Linux, Windows, macOS, Android (via Termux), and more

## Installation

### Quick Install

**Linux / Termux:**
```bash
bash <(curl -sSL "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/scripts/install.sh")
```

**Windows:**
```powershell
powershell -NoExit -ExecutionPolicy Bypass -Command "Invoke-RestMethod -Uri 'https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/scripts/install.ps1' | Invoke-Expression"
```

### Manual Download

Download pre-built binaries for your platform from the [releases page](https://github.com/rhythmcache/payload-dumper-rust/releases).


### Build from Source

If you have Rust installed:
```bash
cargo install payload_dumper
```

## Usage

### Basic Examples

Extract all partitions from a payload file:
```bash
payload_dumper payload.bin --out extracted
```

Extract directly from a ROM ZIP (no need to unzip first):
```bash
payload_dumper ota_update.zip --out extracted
```

Extract from a URL (great for CI/CD or when you don't want to download the whole file):
```bash
payload_dumper https://example.com/ota_update.zip --out extracted
```

Extract specific partitions only:
```bash
payload_dumper payload.bin --images boot,vendor_boot --out extracted
```

List available partitions without extracting:
```bash
payload_dumper payload.bin --list
```

### Advanced Options

```
Usage: payload_dumper [OPTIONS] <PAYLOAD_PATH>

Arguments:
  <PAYLOAD_PATH>              Path to payload.bin, ROM ZIP, or URL

Options:
  --out <OUT>                 Output directory [default: output]
  --images <IMAGES>           Comma-separated partition names to extract
  --list                      List all available partitions
  --threads <THREADS>         Number of parallel threads to use
  --no-parallel               Disable parallel extraction
  --no-verify                 Skip hash verification
  --metadata                  Save complete metadata as JSON
  --diff                      Enable differential OTA mode
  --old <OLD>                 Directory with old partitions (for differential OTA)
  --user-agent <AGENT>        Custom User-Agent for HTTP requests
```

### Practical Examples

Extract boot and vendor_boot from a URL:
```bash
payload_dumper --images boot,vendor_boot https://dl.google.com/path/to/ota.zip
```

Process with custom thread count:
```bash
payload_dumper payload.bin --threads 8 --out output
```

Get metadata without extracting:
```bash
payload_dumper payload.bin --metadata --out metadata.json
```

## How It Works

1. Reads the payload structure from the file/ZIP/URL
2. Identifies all available partitions
3. Decompresses each partition in parallel (unless disabled)
4. Verifies the integrity of extracted files
5. Saves partitions to your output directory


## Technical Details

- **Parallel Processing**: By default, uses all available CPU cores for maximum speed
- **Memory Efficient**: Streams data instead of loading everything into memory
- **Network Optimized**: When extracting from URLs, only downloads required chunks

## Dependencies

- Protocol buffer definitions from [Android's update_engine](https://android.googlesource.com/platform/system/update_engine/+/HEAD/update_metadata.proto)
- See [Cargo.toml](./Cargo.toml) for complete dependency list

## Building from Source

Requirements:
- Rust toolchain (install from [rustup.rs](https://rustup.rs))
- Cargo (comes with Rust)

Build command:
```bash
cargo build --release
```

The binary will be at `target/release/payload_dumper`

## Credits

Inspired by [vm03/payload_dumper](https://github.com/vm03/payload_dumper)

## License

[Apache-2](./LICENSE)

