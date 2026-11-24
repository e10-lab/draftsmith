#!/bin/sh
# 데스크톱 앱(Tauri)을 macOS 앱 묶음(target/DraftSmith.app)으로 만든다. BEATS 의 같은 이름 스크립트와 같은 방식 —
# tauri-cli 없이 실행 파일만 감싼다 (화면 파일은 실행 파일 안에 들어 있다).
set -e
cd "$(dirname "$0")/.."
cargo build -p draftsmith-app --release
# 판은 Cargo.toml 한 곳에만 적는다 (구현 노트 §2)
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/DraftSmith.app
ICONS=crates/draftsmith-app/icons
rm -rf "${APP:?}"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/draftsmith-app "$APP/Contents/MacOS/draftsmith-app"
# 아이콘: icon.png 에서 .icns 를 만든다 (macOS 의 sips·iconutil)
if [ ! -f "$ICONS/icon.icns" ] || [ "$ICONS/icon.png" -nt "$ICONS/icon.icns" ]; then
  SET=$(mktemp -d)/icon.iconset
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    sips -z $s $s "$ICONS/icon.png" --out "$SET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    [ $d -le 512 ] && sips -z $d $d "$ICONS/icon.png" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$SET" -o "$ICONS/icon.icns"
fi
cp "$ICONS/icon.icns" "$APP/Contents/Resources/DraftSmith.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>DraftSmith</string>
  <key>CFBundleDisplayName</key><string>DraftSmith</string>
  <key>CFBundleIdentifier</key><string>kr.draftsmith.app</string>
  <key>CFBundleIconFile</key><string>DraftSmith</string>
  <key>CFBundleExecutable</key><string>draftsmith-app</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>NSHumanReadableCopyright</key><string>© 2026 zero architects. MIT OR Apache-2.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# 서명 — 고정 인증서로 서명해야 macOS 가 빌드마다 같은 앱으로 보아 폴더 접근 허락을 다시 묻지 않는다.
# 기본은 BEATS 에서 이미 만든 「BEATS Local Signing」 인증서를 같이 쓴다 (DRAFTSMITH_SIGN_IDENTITY 로 바꿈).
IDENTITY="${DRAFTSMITH_SIGN_IDENTITY:-BEATS Local Signing}"
if security find-identity -p codesigning 2>/dev/null | grep -q "\"$IDENTITY\"" && codesign --force --sign "$IDENTITY" "$APP"; then
  echo "서명: $IDENTITY"
else
  codesign --force --sign - "$APP"
  echo "서명: 임시(adhoc) — 「$IDENTITY」 인증서가 없거나 쓸 수 없습니다 (BEATS 의 tools/make_signing_cert.sh 로 만들 수 있음)"
fi
echo "만듦: $APP"
