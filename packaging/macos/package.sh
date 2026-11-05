#!/usr/bin/env bash
# Universal app and DMG. Run on a Mac with both release binaries built.
set -euo pipefail
version=${1:?usage: package.sh VERSION [OUTPUT_DIR]}
out=${2:-dist}
root=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
app="$stage/mdview.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create "$root/target/aarch64-apple-darwin/release/mdview" \
    "$root/target/x86_64-apple-darwin/release/mdview" -output "$app/Contents/MacOS/mdview"
lipo "$app/Contents/MacOS/mdview" -verify_arch arm64 x86_64
cp "$root/packaging/macos/Info.plist" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$app/Contents/Info.plist"
mkdir "$stage/icon.iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$root/assets/icon.png" --out "$stage/icon.iconset/icon_${size}x${size}.png" >/dev/null
    twice=$((size * 2))
    sips -z "$twice" "$twice" "$root/assets/icon.png" --out "$stage/icon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$stage/icon.iconset" -o "$app/Contents/Resources/mdview.icns"
cp "$root/README.md" "$root/docs/releases.md" "$app/Contents/Resources/"
identity=${MACOS_SIGN_IDENTITY:--}
if [[ $identity == - ]]; then
    codesign --force --sign - "$app"
else
    codesign --force --options runtime --timestamp --sign "$identity" "$app"
fi
codesign --verify --deep --strict "$app"
"$app/Contents/MacOS/mdview" --help
ln -s /Applications "$stage/Applications"
# Only app + Applications symlink belong in the mounted image.
rm -rf "$stage/icon.iconset"
dmg="$out/mdview-v$version-macos-universal.dmg"
hdiutil create -volname mdview -srcfolder "$stage" -ov -format UDZO "$dmg"
if [[ $identity != - ]]; then
    codesign --timestamp --sign "$identity" "$dmg"
    notary_args=(--keychain-profile mdview-notary)
    if [[ -n ${MACOS_KEYCHAIN:-} ]]; then notary_args+=(--keychain "$MACOS_KEYCHAIN"); fi
    xcrun notarytool submit "$dmg" "${notary_args[@]}" --wait
    xcrun stapler staple "$dmg"
    xcrun stapler validate "$dmg"
fi
# Verify the actual distribution artifact, not just its staging directory.
mount=$(mktemp -d)
hdiutil attach "$dmg" -nobrowse -readonly -mountpoint "$mount"
trap 'hdiutil detach "$mount" >/dev/null || true; rmdir "$mount"; rm -rf "$stage"' EXIT
codesign --verify --deep --strict "$mount/mdview.app"
"$mount/mdview.app/Contents/MacOS/mdview" --help
if [[ $identity != - ]]; then
    spctl --assess --type execute --verbose "$mount/mdview.app"
fi
