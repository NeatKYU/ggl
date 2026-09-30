#!/bin/sh
# ggl 설치:  curl -fsSL https://raw.githubusercontent.com/NeatKYU/ggl/main/install.sh | sh
# 최신 릴리스를 받아 ggl.app 과 터미널 명령(ggl-open)을 설치한다.
# curl로 받은 파일에는 macOS 격리 표시(quarantine)가 붙지 않아서,
# 브라우저로 받았을 때 뜨는 "확인되지 않은 개발자" 경고 없이 바로 열린다.
#   GGL_APP_DIR=폴더   앱을 둘 곳 (기본: /Applications, 쓸 수 없으면 ~/Applications)
#   GGL_BIN_DIR=폴더   ggl-open 을 둘 곳 (기본: ~/.local/bin)
set -eu

REPO="NeatKYU/ggl"
BIN_DIR="${GGL_BIN_DIR:-$HOME/.local/bin}"
if [ -n "${GGL_APP_DIR:-}" ]; then
  APP_DIR="$GGL_APP_DIR"
elif [ -w /Applications ]; then
  APP_DIR=/Applications
else
  APP_DIR="$HOME/Applications"
fi

if [ "$(uname -s)" != Darwin ]; then
  echo "ggl은 macOS 전용이에요." >&2
  exit 1
fi

URL="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep -o 'https://[^"]*macos-universal\.zip' | head -n 1 || true)"
if [ -z "$URL" ]; then
  echo "최신 릴리스를 찾지 못했어요: https://github.com/$REPO/releases" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "받는 중: $URL"
curl -fsSL -o "$TMP/ggl.zip" "$URL"
# ditto는 .app 안의 서명·권한을 그대로 보존한다
ditto -x -k "$TMP/ggl.zip" "$TMP"
curl -fsSL -o "$TMP/ggl-open" "https://raw.githubusercontent.com/$REPO/main/scripts/ggl-open"

mkdir -p "$APP_DIR" "$BIN_DIR"
rm -rf "$APP_DIR/ggl.app"
ditto "$TMP/ggl.app" "$APP_DIR/ggl.app"
install -m 755 "$TMP/ggl-open" "$BIN_DIR/ggl-open"

echo "설치: $APP_DIR/ggl.app"
echo "설치: $BIN_DIR/ggl-open  (터미널에서 'ggl-open .')"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "참고: $BIN_DIR 이 PATH에 없어요. 셸 설정에 추가하면 ggl-open 을 쓸 수 있어요." ;;
esac
