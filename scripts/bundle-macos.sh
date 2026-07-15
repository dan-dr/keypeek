#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    echo "Usage: scripts/bundle-macos.sh [--install]"
    exit 0
fi
if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--install" ) ]]; then
    echo "Usage: scripts/bundle-macos.sh [--install]" >&2
    exit 2
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

cargo bundle --release --format osx

app="$repo_root/target/release/bundle/osx/KeyPeek.app"
identity="${KEYPEEK_CODESIGN_IDENTITY:?Set KEYPEEK_CODESIGN_IDENTITY to a stable signing certificate name}"
if [[ "$identity" == "-" ]]; then
    echo "KEYPEEK_CODESIGN_IDENTITY must be a stable certificate, not ad-hoc signing (-)" >&2
    exit 2
fi

/usr/bin/codesign --force --sign "$identity" "$app"
/usr/bin/codesign --verify --strict --verbose=2 "$app"

if [[ "${1:-}" == "--install" ]]; then
    install_dir="${HOME:?HOME is not set}/Applications"
    installed_app="$install_dir/KeyPeek.app"
    mkdir -p "$install_dir"

    if [[ -L "$installed_app" ]]; then
        unlink "$installed_app"
    elif [[ -e "$installed_app" ]]; then
        backup_app="$install_dir/KeyPeek.app.backup-$(date +%s)"
        echo "Preserving existing app at $backup_app"
        mv "$installed_app" "$backup_app"
    fi

    ln -s "$app" "$installed_app"
    echo "Installed $installed_app -> $app"
fi

echo "$app"
