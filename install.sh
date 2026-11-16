#!/bin/sh
set -e

REPO="mirairoad/ren"
BIN_DIR="$HOME/.local/bin"
BIN="$BIN_DIR/ren"

LOCAL=""
if [ "${1:-}" = "--local" ]; then
    LOCAL=1
    SRC="$(cd "$(dirname "$0")" && pwd)"
fi

case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) echo "Unsupported system: $(uname -s)"; exit 1 ;;
esac

case "$(uname -m)" in
    x86_64|amd64) arch=x86_64 ;;
    arm64|aarch64) arch=arm64 ;;
    *) echo "Unsupported architecture: $(uname -m)"; exit 1 ;;
esac

dl() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    else
        wget -qO "$2" "$1"
    fi
}

mkdir -p "$BIN_DIR"
if [ -n "$LOCAL" ]; then
    echo "Building ren from $SRC..."
    cargo build --release --manifest-path "$SRC/Cargo.toml"
    cp "$SRC/target/release/ren" "$BIN.tmp"
else
    echo "Downloading ren..."
    # Released: Linux x86_64 and arm64, macOS Apple Silicon. Anything else
    # (an Intel Mac, since 0.4.0) builds from source.
    if ! dl "https://github.com/$REPO/releases/latest/download/ren-$os-$arch" "$BIN.tmp"; then
        rm -f "$BIN.tmp"
        echo "The latest ren release has no binary for $os $arch. Build it from source"
        echo "(needs Rust from https://rustup.rs and a C compiler):"
        echo "  git clone https://github.com/$REPO && cd ren && ./install.sh --local"
        echo "or, for the binary only:"
        echo "  cargo install --git https://github.com/$REPO"
        exit 1
    fi
fi
chmod +x "$BIN.tmp"
mv "$BIN.tmp" "$BIN"
"$BIN" --init || true

ICON_URL="https://raw.githubusercontent.com/$REPO/main/assets/ren-512.png"

icon() {
    if [ -n "$LOCAL" ]; then
        cp "$SRC/assets/ren-512.png" "$1"
    else
        dl "$ICON_URL" "$1"
    fi
}

if [ "$os" = linux ]; then
    ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
    APP_DIR="$HOME/.local/share/applications"
    mkdir -p "$ICON_DIR" "$APP_DIR"
    icon "$ICON_DIR/ren.png" || true

    if command -v ghostty >/dev/null 2>&1; then
        EXEC="ghostty -e $BIN %F"
        IN_TERM=false
    else
        EXEC="$BIN %F"
        IN_TERM=true
    fi

    cat > "$APP_DIR/ren.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Ren IDE
Comment=Minimal terminal IDE
Exec=$EXEC
Icon=$ICON_DIR/ren.png
Terminal=$IN_TERM
Categories=Development;TextEditor;
DESKTOP
    chmod +x "$APP_DIR/ren.desktop"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true

    DESK="$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")"
    if [ -d "$DESK" ]; then
        cp "$APP_DIR/ren.desktop" "$DESK/ren.desktop"
        chmod +x "$DESK/ren.desktop"
        command -v gio >/dev/null 2>&1 && gio set "$DESK/ren.desktop" metadata::trusted true 2>/dev/null || true
    fi
fi

if [ "$os" = macos ]; then
    xattr -d com.apple.quarantine "$BIN" 2>/dev/null || true

    APP="$HOME/Applications/Ren IDE.app"
    rm -rf "$APP"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

    if [ -d "/Applications/Ghostty.app" ] || [ -d "$HOME/Applications/Ghostty.app" ]; then
        LOGIN_SHELL="${SHELL:-/bin/zsh}"
        cat > "$APP/Contents/MacOS/ren-launcher" <<LAUNCH
#!/bin/sh
exec open -na Ghostty --args -e "$LOGIN_SHELL" -lic 'exec "$BIN"'
LAUNCH
    else
        TERM_APP="Terminal"
        [ -d "/Applications/iTerm.app" ] && TERM_APP="iTerm"
        cat > "$APP/Contents/MacOS/ren-launcher" <<LAUNCH
#!/bin/sh
open -a "$TERM_APP" "$BIN"
LAUNCH
    fi
    chmod +x "$APP/Contents/MacOS/ren-launcher"

    cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Ren IDE</string>
    <key>CFBundleDisplayName</key><string>Ren IDE</string>
    <key>CFBundleIdentifier</key><string>dev.ren.ide</string>
    <key>CFBundleExecutable</key><string>ren-launcher</string>
    <key>CFBundleIconFile</key><string>ren</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>1</string>
</dict>
</plist>
PLIST

    TMP="$(mktemp -d)"
    if icon "$TMP/icon.png"; then
        SET="$TMP/ren.iconset"
        mkdir -p "$SET"
        for s in 16 32 128 256 512; do
            sips -z $s $s "$TMP/icon.png" --out "$SET/icon_${s}x${s}.png" >/dev/null
            d=$((s * 2))
            sips -z $d $d "$TMP/icon.png" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
        done
        iconutil -c icns "$SET" -o "$APP/Contents/Resources/ren.icns" || true
    fi
    rm -rf "$TMP"
    touch "$APP"

    if [ -d "$HOME/Desktop" ]; then
        rm -f "$HOME/Desktop/Ren IDE.app"
        ln -s "$APP" "$HOME/Desktop/Ren IDE.app"
    fi
fi

echo ""
echo "ren installed: $BIN"
case ":$PATH:" in
    *":$BIN_DIR:"*) echo "Run it with: ren" ;;
    *)
        echo "Add this line to your ~/.bashrc or ~/.zshrc to use the ren command:"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac
