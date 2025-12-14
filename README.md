# payload-dumper-rust 🦀

A fast and efficient Android OTA payload dumper.


## What it does

Extracts partition images (boot, system, vendor, etc.) from Android OTA `payload.bin` files.

## This has these features

- **Parallel extraction** - Uses all CPU cores for faster extraction
- **Works with ZIP files** - Extract directly from ROM ZIPs without unzipping first
- **URL support** - Extract from remote URLs, downloading only the needed data instead of the entire file
- **Cross-platform** - Works on Linux, Windows, macOS, and Android (Termux)

## Installation

### Quick Install

**Linux / macOS / Termux:**
```bash
bash <(curl -sSL "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/scripts/install.sh")
```

**Windows (PowerShell):**
```powershell
powershell -NoExit -ExecutionPolicy Bypass -Command "Invoke-RestMethod -Uri 'https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/scripts/install.ps1' | Invoke-Expression"
```

### Pre-built Binaries

Download from [releases](https://github.com/rhythmcache/payload-dumper-rust/releases).

### Build from Source

```bash
cargo install payload_dumper
```

Or:
```bash
git clone https://github.com/rhythmcache/payload-dumper-rust
cd payload-dumper-rust
cargo build --release
```

## Usage

### Basic Examples

**Extract from payload.bin:**
```bash
payload_dumper payload.bin -o output
```

**Extract from ROM ZIP** (no unzipping required):
```bash
payload_dumper rom.zip -o output
```

**Extract from URL** (downloads only required data):
```bash
payload_dumper https://example.com/ota.zip -o output
```

**Extract specific partitions:**
```bash
payload_dumper payload.bin -i boot,vendor_boot -o output
```

**List partitions without extracting:**
```bash
payload_dumper payload.bin --list
```

### Advanced Options

**URL extraction with prefetch** (better for slow connections):
```bash
payload_dumper https://example.com/ota.zip --prefetch -o output
```

**Custom thread count:**
```bash
payload_dumper payload.bin -t 8 -o output
```

**Export metadata as JSON:**
```bash
payload_dumper payload.bin --metadata -o output
# Creates output/payload_metadata.json
```

**Full metadata with operation details:**
```bash
payload_dumper payload.bin --metadata=full -o output
```

**Skip verification** (faster but not recommended):
```bash
payload_dumper payload.bin --no-verify -o output
```

### All Options

```
Usage: payload_dumper [OPTIONS] <PAYLOAD_PATH>

Arguments:
  <PAYLOAD_PATH>  Path to payload.bin, ROM ZIP, or URL

Options:
  -o, --out <OUT>              Output directory [default: output]
  -U, --user-agent <AGENT>     Custom User-Agent for HTTP requests
  -i, --images <IMAGES>        Comma-separated partition names to extract
  -t, --threads <THREADS>      Number of threads for parallel processing
  -l, --list                   List available partitions
  -m, --metadata[=<MODE>]      Save metadata as JSON (compact or full)
  -P, --no-parallel            Disable parallel extraction
  -n, --no-verify              Skip hash verification
      --prefetch               Download all data first (for remote URLs)
  -h, --help                   Show help
  -V, --version                Show version
```

## Why extract from URLs?

Instead of downloading a 3GB OTA file to extract a 50MB boot image, this tool downloads only ~50-100MB of required data.

Useful for:
- Quick partition extraction without full downloads
- CI/CD pipelines
- Bandwidth-limited connections

## Troubleshooting

**"Server doesn't support range requests"**  
Download the file first, then extract locally.

**Out of memory errors**  
Use `--no-parallel` or reduce thread count with `-t`.

**Slow extraction**  
Remove `--no-parallel` if set. For remote extraction, try `--prefetch`.

## Credits

Inspired by [vm03/payload_dumper](https://github.com/vm03/payload_dumper)

## License

[Apache-2.0](./LICENSE)
