#!/bin/bash

REPO_OWNER="rhythmcache"
REPO_NAME="payload-dumper-rust"
GITHUB_API_URL="https://api.github.com/repos/$REPO_OWNER/$REPO_NAME/releases/latest"

# Determine system type and architecture
echo -n "[*] Checking system type... "
system_type="linux"
if echo "$PREFIX" | grep -q "com.termux"; then
    system_type="termux"
    echo "Termux"
    arch=$(getprop ro.product.cpu.abi)
    bin_dir="$PREFIX/bin"
elif [[ "$OSTYPE" == "darwin"* ]]; then
    system_type="darwin"
    echo "macOS"
    arch=$(uname -m)
    bin_dir="$HOME/.extra/bin"
else
    echo "Linux"
    arch=$(uname -m)
    bin_dir="$HOME/.extra/bin"
fi

echo "[*] Detected architecture: $arch"
sleep 0.5


get_asset_pattern() {
    local arch=$1
    if echo "$arch" | grep -qiE 'arm64|aarch64'; then
        echo "aarch64|arm64"
    elif echo "$arch" | grep -qiE 'armv7|armeabi-v7a'; then
        echo "armv7"
    elif echo "$arch" | grep -qiE 'x86_64|amd64'; then
        echo "x86_64"
    elif echo "$arch" | grep -qiE 'i686|x86'; then
        echo "i686"
    elif echo "$arch" | grep -qi 'riscv64'; then
        echo "riscv64"
    else
        echo ""
    fi
}

asset_pattern=$(get_asset_pattern "$arch")

if [ -z "$asset_pattern" ]; then
    echo "[!] Unsupported architecture: $arch"
    exit 1
fi

echo "[*] Fetching latest release information..."
sleep 0.5

release_info=$(curl -s "$GITHUB_API_URL")
if [ $? -ne 0 ] || [ -z "$release_info" ]; then
    echo "[✗] Failed to fetch release information"
    exit 1
fi

release_tag=$(echo "$release_info" | grep -o '"tag_name": *"[^"]*"' | cut -d'"' -f4)
echo "[*] Latest release: $release_tag"

echo "[*] Looking for release matching architecture: $arch ($asset_pattern)"
sleep 0.5

assets=$(echo "$release_info" | grep -A 3 '"browser_download_url":\|"name":' | grep -E '"name":|"browser_download_url":')

download_url=""
asset_name=""

while IFS= read -r line; do
    if echo "$line" | grep -q '"name":'; then
        current_name=$(echo "$line" | cut -d'"' -f4)
        if [[ "$system_type" == "termux" ]] && \
           echo "$current_name" | grep -qi "android" && \
           echo "$current_name" | grep -qiE "$asset_pattern"; then
            asset_name="$current_name"
        elif [[ "$system_type" == "darwin" ]] && \
             echo "$current_name" | grep -qi "darwin" && \
             echo "$current_name" | grep -qiE "$asset_pattern"; then
            asset_name="$current_name"
        elif [[ "$system_type" == "linux" ]] && \
             echo "$current_name" | grep -qi "linux" && \
             echo "$current_name" | grep -qiE "$asset_pattern"; then
            asset_name="$current_name"
        fi
    elif echo "$line" | grep -q '"browser_download_url":' && [ -n "$asset_name" ]; then
        download_url=$(echo "$line" | cut -d'"' -f4)
        break
    fi
done <<< "$assets"

if [ -z "$download_url" ] || [ -z "$asset_name" ]; then
    echo "[✗] No matching release found for architecture: $arch"
    echo "[*] Available assets:"
    echo "$release_info" | grep '"name":' | cut -d'"' -f4 | sed 's/^/    /'
    exit 1
fi

echo "[*] Found matching release: $asset_name"
echo "[*] Download URL: $download_url"
temp_dir=$(mktemp -d)
zip_file="$temp_dir/$asset_name"
bin_path="$bin_dir/payload_dumper"

# Create bin directory if it doesn't exist
mkdir -p "$bin_dir"

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
        if [[ "$system_type" != "termux" ]]; then
            echo "   Installed package \`payload_dumper $release_tag\` (executable \`$bin_path\`)"
            echo "   Please add the following to your shell configuration file:"
            echo "   export PATH=\"\$PATH:$bin_dir\""
        else
            echo "   Installed package \`payload_dumper $release_tag\` (executable \`payload_dumper\`)"
        fi
    else
    echo "[✗] Something went wrong. The binary may not be compatible."
    echo "[*] Cleaning up...."
    rm -f "$bin_path"
    exit 1
fi
else
    echo "[✗] Failed to install binary"
    rm -rf "$temp_dir"
    exit 1
fi
