#!/bin/bash
echo -n "[*] Checking if running in Termux... "
if ! echo "$PREFIX" | grep -q "com.termux"; then
    echo "✗"
    echo "[!] Not running in Termux. Exiting."
    exit 1
else
    echo "✓"
fi
arch=$(getprop ro.product.cpu.abi)
echo "[*] Detected architecture: $arch"
sleep 0.5
supported_archs="arm64-v8a armeabi-v7a x86 x86_64"
if ! echo "$supported_archs" | grep -qw "$arch"; then
    echo "[!] Unsupported architecture: $arch"
    exit 1
fi
bin_path="$PREFIX/bin/payload_dumper"
url="https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/module/uncommon/payload_dumper-$arch"
echo "[*] Downloading payload_dumper binary..."
sleep 0.5
if curl --silent --show-error -Lo "$bin_path" "$url"; then
    chmod +x "$bin_path"
    echo "[✓] Installed successfully"
else
    echo "[✗] Download failed. Please check your connection."
    exit 1
fi
echo "[*] Verifying the binary works..."
sleep 0.5
if "$bin_path" --help >/dev/null 2>&1; then
    echo "[✓] Success"
else
    echo "[✗] Something went wrong. The binary may not be compatible."
    exit 1
fi
echo
echo "[✔] Setup complete!"
echo "You can now run it using payload_dumper"
