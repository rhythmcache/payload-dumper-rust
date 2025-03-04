#!/data/data/com.termux/files/usr/bin/bash
curr=$(pwd)
if [ -z "$PREFIX" ] || [ ! -d "$PREFIX" ]; then
  echo "This script must be run in Termux. Exiting."
  exit 1
fi

echo "Running in Termux. Proceeding..."
sleep 2

install_package() {
  for pkg in "$@"; do
    if ! command -v "$pkg" &> /dev/null; then
      echo "Installing $pkg..."
      pkg install -y "$pkg"
    else
      echo "$pkg is already installed."
    fi
  done
}

install_package rust cmake zlib-static zlib pkg-config libzip liblzma liblzma-static protobuf

[ -d "$curr/payload-dumper-rust" ] && rm -r "$curr/payload-dumper-rust"
mkdir -p "$curr/payload-dumper-rust/src" "$curr/payload-dumper-rust/proto"
curl -L -o "$curr/payload-dumper-rust/Cargo.toml" "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/Cargo.toml"
curl -L -o "$curr/payload-dumper-rust/src/main.rs" "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/src/main.rs"
curl -L -o "$curr/payload-dumper-rust/proto/update_metadata.proto" "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/proto/update_metadata.proto"
curl -L -o "$curr/payload-dumper-rust/build.rs" "https://raw.githubusercontent.com/rhythmcache/payload-dumper-rust/main/build.rs"

cd payload-dumper-rust && cargo build --release
payload_dumper_rust="$curr/payload-dumper-rust/target/release/payload_dumper"
[ -f "$payload_dumper_rust" ] && sleep 1 && echo "" && echo "- Build Completed" && sleep 0.5 && echo "- adding to path" && cp "$payload_dumper_rust" "$PREFIX/bin"/ && chmod +x "$PREFIX/bin/payload_dumper" && echo "- Installation Complete!!!" && sleep 1 && echo "- You can use it by running payload_dumper" && echo "- Cleaning up" && rm -rf "$curr/payload-dumper-rust" && echo "- Done"







