#!/usr/bin/env bash
# bootstrap.sh — build and install aidp (macOS / Linux)
# Build aidp, install it into a stable folder (~/.aidp/bin, override with AIDP_INSTALL_DIR) and
# put that folder on PATH via the shell rc. The skill's setup script runs this, then installs hooks.
# Idempotent: safe to re-run.

set -e

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$repo_root/scripts/aidp-install.sh"
release_dir="${CARGO_TARGET_DIR:-$repo_root/target}/release"
built_aidp="$release_dir/aidp"
install_dir="${AIDP_INSTALL_DIR:-$HOME/.aidp/bin}"

section() { printf '\n=== %s ===\n' "$1"; }
ok()    { printf '  OK    %s\n' "$1"; }
skip()  { printf '  SKIP  %s\n' "$1"; }
fail()  { printf '  FAIL  %s\n' "$1"; }

# 1. cargo
section '1/3  Toolchain check'
if ! command -v cargo >/dev/null 2>&1; then
    fail 'cargo not found. Install Rust from https://rustup.rs and re-run this script.'
    exit 1
fi
ok "cargo at $(command -v cargo)"

# 2. Build and install aidp
section '2/3  Build and install aidp'
( cd "$repo_root" && cargo build --release -p aidp-cli )
if [ ! -x "$built_aidp" ]; then
    fail "build succeeded but $built_aidp not found"
    exit 1
fi
ok "binary at $built_aidp"
if ! installed="$(aidp_install_binary "$built_aidp" "$install_dir")"; then
    fail "could not install aidp into $install_dir"
    exit 1
fi
ok "installed $installed (copied from $built_aidp)"

# 3. Put the install folder on PATH via shell rc (idempotent, migrates old entry)
section '3/3  PATH'
shell_name="$(basename "${SHELL:-/bin/bash}")"
case "$shell_name" in
    zsh)  rc_file="$HOME/.zshrc" ;;
    bash) rc_file="$HOME/.bashrc"; [ -f "$HOME/.bash_profile" ] && rc_file="$HOME/.bash_profile" ;;
    fish) rc_file="$HOME/.config/fish/config.fish" ;;
    *)    rc_file="$HOME/.profile" ;;
esac

result="$(aidp_migrate_rc "$rc_file" "$install_dir" "$shell_name")"
case "$result" in
    migrated) ok "replaced the old target/release entry in $rc_file with $install_dir" ;;
    added)    ok "appended $install_dir to $rc_file (new terminals will see it)" ;;
    *)        skip "$install_dir already declared in $rc_file" ;;
esac
export PATH="$install_dir:$PATH"

printf '\n'
"$installed" --version
printf 'aidp installed. Set up the skill with /pipeline-setup-v7, or open a new terminal to use aidp.\n'
