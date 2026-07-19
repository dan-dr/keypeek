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
    install_dir="/Applications"
    installed_app="$install_dir/KeyPeek.app"
    legacy_app="${HOME:?HOME is not set}/Applications/KeyPeek.app"
    trash_dir="$HOME/.Trash"
    lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

    if [[ -L "$legacy_app" ]]; then
        /usr/bin/unlink "$legacy_app"
        echo "Removed legacy symlink at $legacy_app"
    elif [[ -e "$legacy_app" ]]; then
        legacy_backup="$trash_dir/KeyPeek-legacy.app.backup-$(date +%s)"
        echo "Moving legacy app to Trash at $legacy_backup"
        mv "$legacy_app" "$legacy_backup"
    fi

    if [[ -L "$installed_app" ]]; then
        /usr/bin/unlink "$installed_app"
    elif [[ -e "$installed_app" ]]; then
        backup_app="$trash_dir/KeyPeek.app.backup-$(date +%s)"
        echo "Moving existing app to Trash at $backup_app"
        mv "$installed_app" "$backup_app"
    fi

    /usr/bin/ditto "$app" "$installed_app"
    /usr/bin/codesign --verify --strict --verbose=2 "$installed_app"
    "$lsregister" -f "$installed_app"
    /usr/bin/mdimport "$installed_app" >/dev/null 2>&1 || true
    echo "Installed $installed_app"
fi

echo "$app"
