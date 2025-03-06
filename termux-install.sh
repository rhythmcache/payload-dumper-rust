#!/bin/bash
echo "$PREFIX" | grep -q "com.termux" || { echo "Not running in Termux"; exit 1; }
arch=$(getprop ro.product.cpu.abi)
bin_path="$PREFIX/bin/payload_dumper"
url="https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/module/uncommon/payload_dumper-$arch"
echo "arm64-v8a armeabi-v7a x86 x86_64" | grep -qw "$arch" || { echo "Unsupported arch: $arch"; exit 1; }
echo "Downloading $arch binary..."
curl -Lo "$bin_path" "$url" && chmod +x "$bin_path" && echo "Installed at $bin_path" || { echo "Download failed"; exit 1; }
