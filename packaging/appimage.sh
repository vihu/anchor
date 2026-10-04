#!/usr/bin/env bash
# Builds target/dist/anchor-<version>-<arch>.AppImage for this machine's
# architecture (x86_64 or aarch64).
#
# anchor links no mpv: it runs the mpv installed on the host, so the
# AppImage carries only anchor. linuxdeploy leaves out what every system
# has (glibc, fontconfig, freetype, the graphics driver), so it runs where
# glibc is at least as new as the build machine's. Downloads linuxdeploy,
# appimagetool and the AppImage runtime, pinned and checked below, into
# target/appimage on first use. The same tools and versions as itele's.
set -euo pipefail
cd "$(dirname "$0")/.."

id=io.github.vihu.anchor
arch=$(uname -m)
version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml)
deploy_version=1-alpha-20251107-1
tool_version=1.9.1
runtime_version=20251108
case $arch in
x86_64)
  deploy_sha256=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
  tool_sha256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
  runtime_sha256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
  ;;
aarch64)
  deploy_sha256=620095110d693282b8ebeb244a95b5e911cf8f65f76c88b4b47d16ae6346fcff
  tool_sha256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
  runtime_sha256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
  ;;
*)
  echo "no AppImage tooling for $arch" >&2
  exit 1
  ;;
esac

# Downloads $2 to $1 unless it is already there, then checks it against $3.
fetch() {
  if [ ! -f "$1" ]; then
    curl -fsSL -o "$1.part" "$2"
    mv "$1.part" "$1"
  fi
  echo "$3  $1" | sha256sum --check --quiet
}

cache=target/appimage
mkdir -p "$cache"
deploy=$cache/linuxdeploy-$deploy_version-$arch.AppImage
tool=$cache/appimagetool-$tool_version-$arch.AppImage
runtime=$cache/runtime-$runtime_version-$arch
fetch "$deploy" "https://github.com/linuxdeploy/linuxdeploy/releases/download/$deploy_version/linuxdeploy-$arch.AppImage" "$deploy_sha256"
fetch "$tool" "https://github.com/AppImage/appimagetool/releases/download/$tool_version/appimagetool-$arch.AppImage" "$tool_sha256"
fetch "$runtime" "https://github.com/AppImage/type2-runtime/releases/download/$runtime_version/runtime-$arch" "$runtime_sha256"
chmod +x "$deploy" "$tool"

cargo build --profile dist --locked

appdir=$(mktemp -d)/anchor.AppDir
# Extract-and-run: build machines (CI runners, containers) often lack FUSE.
APPIMAGE_EXTRACT_AND_RUN=1 "$deploy" --appdir "$appdir" \
  --executable target/dist/anchor \
  --desktop-file "packaging/$id.desktop" \
  --icon-file "packaging/$id.svg"
# appimagetool looks for the metainfo under its older name.
install -Dm644 "packaging/$id.metainfo.xml" "$appdir/usr/share/metainfo/$id.appdata.xml"

mkdir -p target/dist
ARCH=$arch APPIMAGE_EXTRACT_AND_RUN=1 "$tool" --runtime-file "$runtime" \
  "$appdir" "target/dist/anchor-$version-$arch.AppImage"
