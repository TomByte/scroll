#!/bin/sh
# Builds Scroll.app and Scroll.dmg for macOS in dist/.
#
#   scripts/bundle-macos.sh VERSION [TARGET...]
#
# VERSION is what the app reports (1.2.3). Each TARGET is a Rust target triple; with more than
# one the binary is universal. With none, the Mac's own architecture is built.
#
# The app is signed ad hoc, not with a Developer ID, so on another Mac the first launch needs
# right-click > Open (or System Settings > Privacy & Security > Open Anyway).
set -eu

version="${1:?usage: scripts/bundle-macos.sh VERSION [TARGET...]}"
shift
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target_dir="${CARGO_TARGET_DIR:-target}"
dist="$root/dist"
app="$dist/Scroll.app"

binaries=""
if [ "$#" -eq 0 ]; then
    cargo build --release -p scroll-app
    binaries="$target_dir/release/Scroll"
else
    for target in "$@"; do
        cargo build --release -p scroll-app --target "$target"
        binaries="$binaries $target_dir/$target/release/Scroll"
    done
fi

rm -rf "$dist"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
# shellcheck disable=SC2086 # one path per binary
lipo -create $binaries -output "$app/Contents/MacOS/Scroll"
cp crates/scroll-app/assets/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
sed "s/@VERSION@/$version/g" crates/scroll-app/assets/Info.plist > "$app/Contents/Info.plist"
codesign --force --sign - "$app"

# The disk image shows the app beside a link to Applications, to drag one onto the other.
image_root="$dist/image"
mkdir -p "$image_root"
cp -R "$app" "$image_root/"
ln -s /Applications "$image_root/Applications"
hdiutil create -volname Scroll -srcfolder "$image_root" -ov -format UDZO "$dist/Scroll.dmg"
rm -rf "$image_root"
echo "$dist/Scroll.dmg"
