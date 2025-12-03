# payload_dumper
## Dependencies

**Required:**
- liblzma
- libzstd  
- bzip2
- libprotobuf-c

**Optional:** (auto detect)
- libcurl (for HTTP support)
- protoc-c (for regenerating protobuf files)

## Building

### Build

```bash
mkdir -p build && cd build
meson setup ..
ninja
```

## Usage

```bash
Usage: ./payload_dumper <payload_source> [options]
Sources:
  <file_path>          Local payload.bin or ZIP file
  <http_url>           Remote ZIP file URL
Options:
  --out <dir>          Output directory (default: output)
  --images <list>      Comma-separated list of images to extract
  --list               List all partitions and exit
  --threads <num>      Number of threads to use
  --user-agent <ua>    Custom User-Agent for HTTP requests
  --help               Show this help message
```
