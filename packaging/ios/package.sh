#!/usr/bin/env bash
# Build VectorCraft for iPadOS 16+ and package it as a TrollStore .tipa (an IPA, ad-hoc signed;
# TrollStore applies its own signature on install).
#
# Needs: Xcode (iPhoneOS SDK), rustup with the aarch64-apple-ios target.
#
#   packaging/ios/package.sh               -> dist/VectorCraft-<version>-ios16.tipa
#   packaging/ios/package.sh --simulator   -> dist/simulator/VectorCraft.app (Apple silicon Macs)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
export IPHONEOS_DEPLOYMENT_TARGET=16.0
TARGET=aarch64-apple-ios
SIMULATOR=0
if [ "${1:-}" = "--simulator" ]; then
  TARGET=aarch64-apple-ios-sim
  SIMULATOR=1
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
SHORT_VERSION="${VERSION%%-*}"
BUILD_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"

rustup target add "$TARGET" >/dev/null
cargo build --release --locked --target "$TARGET" -p vectorcraft

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
APP="$STAGE/Payload/VectorCraft.app"
mkdir -p "$APP"
cp "target/$TARGET/release/vectorcraft" "$APP/VectorCraft"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" -e "s/@BUILD_SHA@/$BUILD_SHA/g" \
  packaging/ios/Info.plist.in >"$APP/Info.plist"
plutil -lint "$APP/Info.plist" >/dev/null
plutil -convert binary1 "$APP/Info.plist"
printf 'APPL????' >"$APP/PkgInfo"

# Loose iPad icons (no asset catalog): 76 pt and 83.5 pt at 2x.
ICON=assets/app-icon/vectorcraft-1024.png
sips -z 152 152 "$ICON" --out "$APP/AppIcon76x76@2x~ipad.png" >/dev/null
sips -z 167 167 "$ICON" --out "$APP/AppIcon83.5x83.5@2x~ipad.png" >/dev/null
sips -z 76 76 "$ICON" --out "$APP/AppIcon76x76~ipad.png" >/dev/null

# Licences ship with the app.
cp LICENSE-MIT LICENSE-APACHE NOTICE "$APP/"

strip -x "$APP/VectorCraft" 2>/dev/null || true

if [ "$SIMULATOR" = 1 ]; then
  # Simulators of iOS 27 and later stop apps linked against their SDK that don't adopt the UIScene
  # life cycle, which winit 0.30 predates: the binary says it was linked against iOS 18 instead.
  # Only the simulator build: iOS 16 on a device doesn't make that check.
  xcrun vtool -set-build-version iossim 16.0 18.0 -replace -output "$APP/VectorCraft.tmp" "$APP/VectorCraft"
  mv -f "$APP/VectorCraft.tmp" "$APP/VectorCraft"
  codesign --force --sign - --timestamp=none "$APP"
  rm -rf dist/simulator && mkdir -p dist/simulator
  cp -R "$APP" dist/simulator/
  echo "$ROOT/dist/simulator/VectorCraft.app"
  exit 0
fi
codesign --force --sign - --timestamp=none --entitlements packaging/ios/entitlements.plist "$APP"

mkdir -p dist
OUT="$ROOT/dist/VectorCraft-$VERSION-ios16.tipa"
rm -f "$OUT"
(cd "$STAGE" && zip -qry "$OUT" Payload)
echo "$OUT"
