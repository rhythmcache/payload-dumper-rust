# payload-dumper-gui

Windows and Android app to extract Android OTA payloads from local files or HTTP URLs.

---

## Features
- Supports **payload.bin** and **OTA ZIP** files
- Extract **specific partitions directly from remote HTTP OTA URLs** without downloading the full OTA
- Extract multiple partitions simultaneously
- Cancel running extractions
- Optional limit on concurrent extractions to control CPU and I/O usage
- **SHA-256** checksum verification for extracted partitions

---

## Screenshots
### Android

<p align="center">
  <img src="./images/image3.png" width="30%">
  <img src="./images/image2.png" width="30%">
  <img src="./images/image1.png" width="30%">
</p>

### Windows

![Windows screenshot](./images/windows.png)

---

## Limitations
- Currently available only for **Windows** and **Android**
- **Multi-extent OTAs** and **differential OTAs** are not supported
- The remote server must support **HTTP Range requests** for extraction from URLs

---

## Dependencies
### External dependencies used in this project
- [payload-dumper-rust](https://github.com/rhythmcache/payload-dumper-rust.git)  
  Backend library powering the core functionality of both Windows and Android versions

- [json.h](https://github.com/sheredom/json.h.git)  
  Single-header JSON parser library

- [digest](https://github.com/rhythmcache/digest.git)  
  Single-header implementation of **SHA-256**

- [imgui](https://github.com/ocornut/imgui.git)  
  Immediate-mode graphical user interface for C++

If you prefer a command-line utility, use  
[payload-dumper-rust](https://github.com/rhythmcache/payload-dumper-rust.git), which provides the same or more features.

---

## License
- [Apache-2.0](./LICENSE)
