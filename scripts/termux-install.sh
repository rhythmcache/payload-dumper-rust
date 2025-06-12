#!/bin/bash

REPO_OWNER="rhythmcache"
REPO_NAME="payload-dumper-rust"
GITHUB_API_URL="https://api.github.com/repos/$REPO_OWNER/$REPO_NAME/releases/latest"
echo -n "[*] Checking env... "
if ! echo "$PREFIX" | grep -q "com.termux"; then
    echo "✗"
    echo "[!] Not running in Termux. Exiting."
    exit 1
else
    echo "[*] Success"
fi
arch=$(getprop ro.product.cpu.abi)
echo "[*] Detected architecture: $arch"
sleep 0.5
supported_archs="arm64-v8a armeabi-v7a x86 x86_64"
if ! echo "$supported_archs" | grep -qw "$arch"; then
    echo "[!] Unsupported architecture: $arch"
    exit 1
fi


echo "[*] Fetching latest release information..."
sleep 0.5

release_info=$(curl -s "$GITHUB_API_URL")
if [ $? -ne 0 ] || [ -z "$release_info" ]; then
    echo "[✗] Failed to fetch release information from GitHub API"
    exit 1
fi


release_tag=$(echo "$release_info" | grep -o '"tag_name": *"[^"]*"' | cut -d'"' -f4)
echo "[*] Latest release: $release_tag"


echo "[*] Looking for Android release matching architecture: $arch"
sleep 0.5


assets=$(echo "$release_info" | grep -A 3 '"browser_download_url":\|"name":' | grep -E '"name":|"browser_download_url":')


download_url=""
asset_name=""

while IFS= read -r line; do
    if echo "$line" | grep -q '"name":'; then
        current_name=$(echo "$line" | cut -d'"' -f4)
        if echo "$current_name" | grep -qi "android" && echo "$current_name" | grep -q "$arch"; then
            asset_name="$current_name"
        fi
    elif echo "$line" | grep -q '"browser_download_url":' && [ -n "$asset_name" ]; then
        download_url=$(echo "$line" | cut -d'"' -f4)
        break
    fi
done <<< "$assets"

if [ -z "$download_url" ] || [ -z "$asset_name" ]; then
    echo "[✗] No matching Android release found for architecture: $arch"
    echo "[*] Available assets:"
    echo "$release_info" | grep '"name":' | cut -d'"' -f4 | sed 's/^/    /'
    exit 1
fi

echo "[*] Found matching release: $asset_name"
echo "[*] Download URL: $download_url"
temp_dir=$(mktemp -d)
zip_file="$temp_dir/$asset_name"
bin_path="$PREFIX/bin/payload_dumper"

echo "[*] Downloading release archive..."
sleep 0.5

if curl --silent --show-error -Lo "$zip_file" "$download_url"; then
    echo "[✓] Download completed"
else
    echo "[✗] Download failed. Please check your connection."
    rm -rf "$temp_dir"
    exit 1
fi

echo "[*] Extracting archive..."
sleep 0.5

if unzip -q "$zip_file" -d "$temp_dir"; then
    echo "[✓] Archive extracted successfully"
else
    echo "[✗] Failed to extract archive"
    rm -rf "$temp_dir"
    exit 1
fi
binary_file=$(find "$temp_dir" -type f -executable | head -n 1)

if [ -z "$binary_file" ]; then
    binary_file=$(find "$temp_dir" -type f \( -name "payload_dumper*" -o -name "*dumper*" -o -name "*" \) | grep -v "\.zip$" | head -n 1)
fi

if [ -z "$binary_file" ]; then
    echo "[✗] No binary file found in the extracted archive"
    echo "[*] Contents of extracted archive:"
    find "$temp_dir" -type f | sed 's/^/    /'
    rm -rf "$temp_dir"
    exit 1
fi

echo "[*] Found binary: $(basename "$binary_file")"
echo "  Installing $bin_path"
if cp "$binary_file" "$bin_path" && chmod +x "$bin_path"; then
    rm -rf "$temp_dir"
    
    echo "[*] Verifying the binary..."
    sleep 0.5
    
    if "$bin_path" --help >/dev/null 2>&1; then
        echo "   Installed package \`payload_dumper $release_tag\` (executable \`payload_dumper\`)"
    else
        echo "[✗] Something went wrong. The binary may not be compatible."
        exit 1
    fi
else
    echo "[✗] Failed to install binary"
    rm -rf "$temp_dir"
    exit 1
fi
