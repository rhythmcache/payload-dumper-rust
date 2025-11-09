# payload-dumper-rust 🦀

A fast and efficient Android OTA payload dumper written in Rust.

## What does this do?

Ever downloaded an Android OTA update or custom ROM and needed to extract the system images from it? That's what this tool does. Android OTA files contain a `payload.bin` that packs all the partition images (like boot, system, vendor, etc.). This tool unpacks them so you can flash, modify, or analyze them.

## Why should you use this?

- **Fast**: Parallel extraction uses all your CPU cores
- **Memory efficient**: Streams data instead of loading everything into RAM
- **Network smart**: Extract directly from URLs without downloading the whole file
- **Flexible**: Works with payload.bin files, ROM ZIPs, or direct URLs
- **Reliable**: Verifies extracted partitions to catch corruption
- **Cross-platform**: Linux, Windows, macOS, even Android (Termux)

## Installation

### Quick Install (Recommended)

**Linux / macOS / Termux:**
```bash
bash <(curl -sSL "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/scripts/install.sh")
```

**Windows (PowerShell):**
```powershell
powershell -NoExit -ExecutionPolicy Bypass -Command "Invoke-RestMethod -Uri 'https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/scripts/install.ps1' | Invoke-Expression"
```

### Download Pre-built Binaries

Grab the latest binary for your platform from [releases](https://github.com/rhythmcache/payload-dumper-rust/releases).

### Build from Source

Have Rust installed? Just run:
```bash
cargo install payload_dumper
```

Or clone and build:
```bash
git clone https://github.com/rhythmcache/payload-dumper-rust
cd payload-dumper-rust
cargo build --release
```

Binary will be at `target/release/payload_dumper`

## Usage

### Simple Examples

**Extract everything from a payload file:**
```bash
payload_dumper payload.bin -o extracted
```

**Extract from a ROM ZIP** (no need to unzip first!):
```bash
payload_dumper miui_update.zip -o extracted
```

**Extract from a URL** (downloads only what's needed):
```bash
payload_dumper https://example.com/ota_update.zip -o extracted
```

**Only extract specific partitions:**
```bash
payload_dumper payload.bin -i boot,vendor_boot -o extracted
```

**List what's inside without extracting:**
```bash
payload_dumper payload.bin --list
```

### Real-World Use Cases

**Need just the boot image from a URL?** (No need to download 3GB!)
```bash
payload_dumper https://dl.google.com/ota/pixel_ota.zip -i boot -o boot_only
```
This will only download the parts containing the boot partition (~50-100MB) instead of the entire 3GB file.

**Extract with custom thread count:**
```bash
payload_dumper payload.bin -t 8 -o output
```

**Get metadata in JSON format:**
```bash
payload_dumper payload.bin --metadata -o metadata_dir
# Creates metadata_dir/payload_metadata.json (~100KB)
```

**Get FULL metadata** (includes all operation details, can be large):
```bash
payload_dumper payload.bin --metadata=full -o metadata_dir
# Creates metadata_dir/payload_metadata.json (~1000KB)
```

**Skip verification** (faster but risky):
```bash
payload_dumper payload.bin --no-verify -o output
```

**Sequential extraction** (for debugging or low-memory systems):
```bash
payload_dumper payload.bin --no-parallel -o output
```

**Custom User-Agent for URLs:**
```bash
payload_dumper https://example.com/ota.zip --user-agent "MyBot/1.0" -o output
```

### All Options

```
Usage: payload_dumper [OPTIONS] <PAYLOAD_PATH>

Arguments:
  <PAYLOAD_PATH>              Path to payload.bin, ROM ZIP file, or direct URL

Options:
  -o, --out <OUT>             Output directory [default: output]
  -i, --images <IMAGES>       Comma-separated partition names to extract
  -t, --threads <THREADS>     Number of threads for parallel processing
  -l, --list                  List all partitions without extracting
  -m, --metadata[=<MODE>]     Save metadata as JSON:
                                --metadata       Compact mode (~100KB)
                                --metadata=full  Full mode with all details
  -P, --no-parallel           Disable parallel extraction
  -n, --no-verify             Skip hash verification (faster but risky)
  -U, --user-agent <AGENT>    Custom User-Agent for HTTP requests
  -h, --help                  Show help
  -V, --version               Show version
```

## How It Works

Point it at a payload file (or ZIP/URL containing one), tell it what you want, and it extracts the partition images for you. Fast and simple.

## Why Extract from URLs?

When you extract from a URL, it only downloads the parts it needs instead of the whole file.

**Example:** Want just the boot image from a 3GB OTA? This tool downloads ~150MB instead of 3GB. Perfect for:
- Quick partition extraction without full downloads
- CI/CD pipelines that need specific images
- Slow internet connections
- Saving bandwidth



## Building from Source

**Requirements:**
- Rust toolchain ([install from rustup.rs](https://rustup.rs))

**Build:**
```bash
cargo build --release
```

**With all features:**
```bash
cargo build --release --all-features
```

**Available features:**
- `remote_zip` - Extract from URLs (enabled by default)
- `local_zip` - Extract from local ZIP files (enabled by default)
- `metadata` - Metadata extraction support (enabled by default)

## Troubleshooting

**"Server doesn't support range requests"**
- Some servers don't support partial downloads. Try downloading the file first.

**Out of memory errors**
- Try `--no-parallel` or reduce `--threads` count.

**Extraction is slow**
- Check if `--no-parallel` is enabled. Remove it for faster extraction.
- For remote extraction, slow speeds might be due to server throttling.

## Credits

- Inspired by [vm03/payload_dumper](https://github.com/vm03/payload_dumper)
- Protocol buffer definitions from [Android's update_engine](https://android.googlesource.com/platform/system/update_engine/+/HEAD/update_metadata.proto)

## License

[Apache-2.0](./LICENSE)

---

**Found a bug?** Open an issue on [GitHub](https://github.com/rhythmcache/payload-dumper-rust/issues).

**Want to contribute?** PRs are welcome!
