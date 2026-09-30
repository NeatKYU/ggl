#!/bin/bash
# macOS 앱(ggl.app)으로 묶는다.
#   ./bundle.sh               → dist/ggl.app (이 맥의 칩용)
#   ./bundle.sh --universal   → Apple Silicon + Intel 겸용
#   ./bundle.sh --zip         → 배포용 dist/ggl-<버전>-macos-<칩>.zip 도 만들기
#   ./bundle.sh --install     → ~/Applications 에 설치 + 터미널 명령(ggl-open) 설치
set -euo pipefail
cd "$(dirname "$0")"

UNIVERSAL=0 ZIP=0 INSTALL=0
for arg in "$@"; do
  case "$arg" in
    --universal) UNIVERSAL=1 ;;
    --zip) ZIP=1 ;;
    --install) INSTALL=1 ;;
    *) echo "알 수 없는 옵션: $arg" >&2; exit 1 ;;
  esac
done

APP="ggl.app"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
export MACOSX_DEPLOYMENT_TARGET=11.0

if [[ $UNIVERSAL == 1 ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/universal
  lipo -create -output target/universal/ggl \
    target/aarch64-apple-darwin/release/ggl target/x86_64-apple-darwin/release/ggl
  BIN=target/universal/ggl
  ARCH=universal
else
  cargo build --release
  BIN=target/release/ggl
  ARCH="$(uname -m)"
fi

rm -rf "dist/$APP"
mkdir -p "dist/$APP/Contents/MacOS" "dist/$APP/Contents/Resources"
cp "$BIN" "dist/$APP/Contents/MacOS/ggl"
cp assets/AppIcon.icns "dist/$APP/Contents/Resources/"
cat > "dist/$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>ggl</string>
  <key>CFBundleDisplayName</key><string>ggl</string>
  <key>CFBundleIdentifier</key><string>io.github.neatkyu.ggl</string>
  <key>CFBundleExecutable</key><string>ggl</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Apple 개발자 서명이 없어도 macOS가 "손상된 앱"으로 보지 않게 로컬(ad-hoc) 서명
codesign --force --deep --sign - "dist/$APP"
echo "만듦: dist/$APP ($ARCH, $(du -sh "dist/$APP" | cut -f1))"

if [[ $ZIP == 1 ]]; then
  ZIPFILE="dist/ggl-$VERSION-macos-$ARCH.zip"
  rm -f "$ZIPFILE"
  # ditto는 .app 안의 서명·권한을 그대로 보존한다 (zip 명령은 깨뜨릴 수 있음)
  ditto -c -k --keepParent "dist/$APP" "$ZIPFILE"
  echo "만듦: $ZIPFILE"
fi

if [[ $INSTALL == 1 ]]; then
  mkdir -p ~/Applications ~/.local/bin
  rm -rf ~/Applications/"$APP"
  cp -R "dist/$APP" ~/Applications/
  install -m 755 scripts/ggl-open ~/.local/bin/ggl-open
  echo "설치: ~/Applications/$APP"
  echo "설치: ~/.local/bin/ggl-open  (터미널에서 'ggl-open .')"
fi
