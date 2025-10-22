#!/bin/bash

REPO_OWNER="rhythmcache"
REPO_NAME="payload-dumper-rust"
GITHUB_API_URL="https://api.github.com/repos/$REPO_OWNER/$REPO_NAME/releases/latest"

get_yes_no() {
    local prompt="$1"
    local response
    while true; do
        echo -n "$prompt (y/n): "
        read -r response
        case "$response" in
            [Yy]|[Yy][Ee][Ss])
                return 0
                ;;
            [Nn]|[Nn][Oo])
                return 1
                ;;
            *)
                echo "Please answer yes (y) or no (n)."
                ;;
        esac
    done
}

extract_version() {
    local input="$1"
    local version
    version=$(echo "$input" | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' | head -1 | sed 's/^v//')
    if [ -z "$version" ]; then
        version=$(echo "$input" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    fi
    echo "$version"
}

if command -v payload_dumper >/dev/null 2>&1; then
    existing_path=$(which payload_dumper)
    echo "[*] payload_dumper is already installed at: $existing_path"
    
    current_version_output=$(payload_dumper --version 2>/dev/null)
    if [ $? -eq 0 ]; then
        current_version=$(extract_version "$current_version_output")
        echo "[*] Current version: $current_version"
        
        echo "[*] Fetching latest release information..."
        release_info=$(curl -s "$GITHUB_API_URL")
        if [ $? -ne 0 ] || [ -z "$release_info" ]; then
            echo "[!] Failed to fetch release information. Cannot compare versions."
            if get_yes_no "[?] Do you still want to proceed with installation?"; then
                echo "[*] Proceeding with installation..."
            else
                echo "[*] Installation cancelled."
                exit 0
            fi
        else
            release_tag=$(echo "$release_info" | grep -o '"tag_name": *"[^"]*"' | cut -d'"' -f4)
            latest_version=$(extract_version "$release_tag")
            echo "[*] Latest version: $latest_version"
            
            if [ "$current_version" = "$latest_version" ]; then
                echo "[*] You already have the latest version installed."
                if get_yes_no "[?] Do you still want to reinstall it?"; then
                    echo "[*] Proceeding with reinstallation..."
                else
                    echo "[*] Installation cancelled."
                    exit 0
                fi
            else
                echo "[*] A newer version is available!"
                echo "[*] Updating from $current_version to $latest_version..."
                
                install_dir=$(dirname "$existing_path")
                if [ -w "$install_dir" ]; then
                    echo "[*] Removing old version..."
                    if rm -f "$existing_path"; then
                        echo "[✓] Old version removed successfully"
                        bin_dir="$install_dir"
                        update_mode=true
                    else
                        echo "[!] Failed to remove old version. You may need elevated permissions."
                        echo "[*] Will install to default location instead."
                        update_mode=false
                    fi
                else
                    echo "[!] No write permission to $install_dir"
                    echo "[*] Will install to default location instead."
                    update_mode=false
                fi
            fi
        fi
    else
        echo "[!] Could not determine current version (--version command failed)"
        if get_yes_no "[?] Do you want to proceed with installation anyway?"; then
            echo "[*] Proceeding with installation..."
        else
            echo "[*] Installation cancelled."
            exit 0
        fi
    fi
fi

if [ -z "$bin_dir" ]; then
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
    elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]] || [[ "$OSTYPE" == "win32" ]] || [[ "$(uname -s)" == MINGW* ]] || [[ "$(uname -s)" == MSYS* ]]; then
        system_type="windows"
        echo "Windows"
        arch=$(uname -m)
        bin_dir="$HOME/.extra/bin"
    else
        echo "Linux"
        arch=$(uname -m)
        bin_dir="$HOME/.extra/bin"
    fi
else
    if echo "$PREFIX" | grep -q "com.termux"; then
        system_type="termux"
        arch=$(getprop ro.product.cpu.abi)
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        system_type="darwin"
        arch=$(uname -m)
    elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]] || [[ "$OSTYPE" == "win32" ]] || [[ "$(uname -s)" == MINGW* ]] || [[ "$(uname -s)" == MSYS* ]]; then
        system_type="windows"
        arch=$(uname -m)
    else
        system_type="linux"
        arch=$(uname -m)
    fi
fi

echo "[*] Detected architecture: $arch"
sleep 0.5

get_asset_pattern() {
    local arch=$1
    if echo "$arch" | grep -qiE 'arm64|aarch64'; then
        echo "aarch64|arm64"
    elif echo "$arch" | grep -qiE 'armv7|armeabi-v7a'; then
        echo "armv7|armeabi-v7a|arm"
    elif echo "$arch" | grep -qiE 'x86_64|amd64'; then
        echo "x86_64|amd64|x64"
    elif echo "$arch" | grep -qiE 'i686|x86'; then
        echo "i686|x86"
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

if [ -z "$release_info" ]; then
    echo "[*] Fetching latest release information..."
    sleep 0.5
    
    release_info=$(curl -s "$GITHUB_API_URL")
    if [ $? -ne 0 ] || [ -z "$release_info" ]; then
        echo "[✗] Failed to fetch release information"
        exit 1
    fi
    
    release_tag=$(echo "$release_info" | grep -o '"tag_name": *"[^"]*"' | cut -d'"' -f4)
fi

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
        elif [[ "$system_type" == "windows" ]] && \
             echo "$current_name" | grep -qiE "windows|msvc|pc-windows" && \
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

if [[ "$system_type" == "windows" ]]; then
    bin_path="$bin_dir/payload_dumper.exe"
else
    bin_path="$bin_dir/payload_dumper"
fi

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

if [[ "$system_type" == "windows" ]]; then
    binary_file=$(find "$temp_dir" -type f -name "*.exe" | head -n 1)
    if [ -z "$binary_file" ]; then
        binary_file=$(find "$temp_dir" -type f \( -name "payload_dumper*" -o -name "*dumper*" \) | head -n 1)
    fi
else
    binary_file=$(find "$temp_dir" -type f -executable | head -n 1)
    if [ -z "$binary_file" ]; then
        binary_file=$(find "$temp_dir" -type f \( -name "payload_dumper*" -o -name "*dumper*" -o -name "*" \) | grep -v "\.zip$" | head -n 1)
    fi
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
        if [ "$update_mode" = true ]; then
            echo "[✓] Successfully updated payload_dumper to $release_tag"
            echo "   Updated executable: $bin_path"
        else
            if [[ "$system_type" != "termux" ]]; then
                echo "   Installed package \`payload_dumper $release_tag\` (executable \`$bin_path\`)"
                if [[ "$system_type" == "windows" ]]; then
                    echo "   Please add the following to your shell configuration file (.bashrc, .zshrc, etc.):"
                    echo "   export PATH=\"\$PATH:$bin_dir\""
                else
                    echo "   Please add the following to your shell configuration file:"
                    echo "   export PATH=\"\$PATH:$bin_dir\""
                fi
            else
                echo "   Installed package \`payload_dumper $release_tag\` (executable \`payload_dumper\`)"
            fi
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
