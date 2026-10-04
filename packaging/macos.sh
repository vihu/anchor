#!/usr/bin/env bash
# Builds anchor.app for macOS, in two steps:
#
#   macos.sh build
#     On a Mac of either architecture, with librsvg (brew install librsvg):
#     anchor.app for this architecture, as target/dist/anchor-<arch>.app.tar.
#     anchor links no mpv (it runs the user's), so the app is one binary.
#
#   macos.sh merge <arm64.app.tar> <x86_64.app.tar>
#     Joins the two into one app for Apple silicon and Intel, ad-hoc
#     signed, as target/dist/anchor-<version>-macos-universal.zip.
#
# Ad-hoc signing seals the bundle: a download then gets macOS's "could not
# verify" prompt, which Privacy & Security > Open Anyway clears, instead
# of being reported as damaged.
set -euo pipefail
cd "$(dirname "$0")/.."

id=io.github.vihu.anchor
version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml)

build() {
  local arch work app iconset size
  arch=$(uname -m)
  export MACOSX_DEPLOYMENT_TARGET=11.0
  cargo build --profile dist --locked

  work=$(mktemp -d)
  app=$work/anchor.app
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp target/dist/anchor "$app/Contents/MacOS/anchor"

  iconset=$work/anchor.iconset
  mkdir "$iconset"
  for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" "packaging/$id.svg" \
      -o "$iconset/icon_${size}x${size}.png"
    rsvg-convert -w "$((size * 2))" -h "$((size * 2))" "packaging/$id.svg" \
      -o "$iconset/icon_${size}x${size}@2x.png"
  done
  iconutil -c icns -o "$app/Contents/Resources/anchor.icns" "$iconset"

  mkdir -p target/dist
  tar -C "$work" -cf "target/dist/anchor-$arch.app.tar" anchor.app
}

merge() {
  local work app minos
  work=$(mktemp -d)
  mkdir "$work/arm64" "$work/x86_64"
  tar -C "$work/arm64" -xf "$1"
  tar -C "$work/x86_64" -xf "$2"
  app=$work/anchor.app
  cp -R "$work/arm64/anchor.app" "$app"
  lipo -create -output "$app/Contents/MacOS/anchor" \
    "$work/arm64/anchor.app/Contents/MacOS/anchor" \
    "$work/x86_64/anchor.app/Contents/MacOS/anchor"

  # The newest macOS either half was built for.
  minos=$(otool -l "$app/Contents/MacOS/anchor" | awk '/minos/ {print $2}' | sort -V | tail -n 1)

  cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>anchor</string>
  <key>CFBundleExecutable</key><string>anchor</string>
  <key>CFBundleIconFile</key><string>anchor</string>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>anchor</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.entertainment</string>
  <key>LSMinimumSystemVersion</key><string>$minos</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
  plutil -lint "$app/Contents/Info.plist"

  codesign --force --sign - "$app"
  codesign --verify --strict --deep "$app"

  mkdir -p target/dist
  ditto -c -k --keepParent "$app" "target/dist/anchor-$version-macos-universal.zip"
  echo "anchor.app runs on macOS $minos or newer"
}

case ${1:-} in
build) build ;;
merge) merge "$2" "$3" ;;
*)
  echo "usage: macos.sh build | macos.sh merge <arm64.app.tar> <x86_64.app.tar>" >&2
  exit 1
  ;;
esac
