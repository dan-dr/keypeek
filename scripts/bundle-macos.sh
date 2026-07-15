#!/usr/bin/env bash
set -euo pipefail

cargo bundle --release --format osx

app="target/release/bundle/osx/KeyPeek.app"
identity="${KEYPEEK_CODESIGN_IDENTITY:?Set KEYPEEK_CODESIGN_IDENTITY to a stable signing certificate name}"
if [[ "$identity" == "-" ]]; then
    echo "KEYPEEK_CODESIGN_IDENTITY must be a stable certificate, not ad-hoc signing (-)" >&2
    exit 2
fi

/usr/bin/codesign --force --deep --sign "$identity" "$app"
/usr/bin/codesign --verify --strict --verbose=2 "$app"

echo "$app"
