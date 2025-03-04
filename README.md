# payload-dumper-rust
Android OTA payload dumper written in Rust


## features
- apart from extracting from payload.bin , it can extract partitions directly from `url` or rom `zip`

- all decompression process are done parallelely.

- Can also extract individual images 



Here are some performance metrics from a **Poco X4 Pro (SD695, 8GB RAM)** running in Termux:

- **Direct Payload Extraction**: Extracting partitions directly from the `payload.bin` took **2 minutes 26 seconds**.
- **ZIP File Extraction**: Extracting partitions directly from the ZIP file took **2 minutes 30 seconds**, just **4 seconds difference**
- It can also extract partition directly from **url** without having you to download the full rom zip file

### Screenshots

- **Direct Payload Extraction**:  
  ![Direct Payload Extraction](./Screenshot_20250304-175502_Termux.png)

- **ZIP File Extraction**:  
  ![ZIP File Extraction](./Screenshot_20250304-175923_Termux.png)

- **Remote URL Extraction**:  
  ![Remote URL Extraction](./Screenshot_20250304-180030_Termux.png)


### Usage :
```
Usage: payload_dumper [OPTIONS] <PAYLOAD_PATH>

Arguments:
  <PAYLOAD_PATH>  
      Path to the payload file.

Options:
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
