arch=$(getprop ro.product.cpu.abi)
ui_print "- Detected Architecture: $arch"
[ ! -f "$MODPATH/uncommon/payload_dumper-$arch" ] && abort "- Error: Unsupported Architecture"
ui_print "- Creating directories"
mkdir -p "$MODPATH/system/bin"
ui_print "- Copying files"
cp "$MODPATH/uncommon/payload_dumper-$arch" "$MODPATH/system/bin/payload_dumper"
rm -rf "$MODPATH/uncommon"
set_perm_recursive "$MODPATH/system/bin" 0 0 0755 0755
