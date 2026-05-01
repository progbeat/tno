#!/bin/sh
set -eu

repo_url="${CANON_REPO_URL:-https://github.com/progbeat/canon}"
raw_base="${CANON_RAW_BASE:-https://raw.githubusercontent.com/progbeat/canon/main}"
cargo_root="${CARGO_INSTALL_ROOT:-$HOME/.local}"
codex_home="${CODEX_HOME:-$HOME/.codex}"

plugin_dir="$codex_home/plugins/canon"
skill_dir="$codex_home/skills/canon"

if ! command -v cargo >/dev/null 2>&1; then
  echo "canon installer: cargo is required" >&2
  exit 1
fi

if [ -n "${CANON_REV:-}" ]; then
  cargo install --git "$repo_url" --rev "$CANON_REV" --root "$cargo_root" --force
else
  cargo install --git "$repo_url" --root "$cargo_root" --force
fi

mkdir -p "$plugin_dir/.codex-plugin" "$plugin_dir/skills/canon" "$skill_dir"

if [ -f ".codex-plugin/plugin.json" ] && [ -f "skills/canon/SKILL.md" ]; then
  cp ".codex-plugin/plugin.json" "$plugin_dir/.codex-plugin/plugin.json"
  cp "skills/canon/SKILL.md" "$plugin_dir/skills/canon/SKILL.md"
  cp "skills/canon/SKILL.md" "$skill_dir/SKILL.md"
else
  if ! command -v curl >/dev/null 2>&1; then
    echo "canon installer: curl is required outside a checkout" >&2
    exit 1
  fi
  curl -fsSL "$raw_base/.codex-plugin/plugin.json" -o "$plugin_dir/.codex-plugin/plugin.json"
  curl -fsSL "$raw_base/skills/canon/SKILL.md" -o "$plugin_dir/skills/canon/SKILL.md"
  cp "$plugin_dir/skills/canon/SKILL.md" "$skill_dir/SKILL.md"
fi

echo "Installed canon CLI to $cargo_root/bin/canon"
echo "Installed canon plugin bundle to $plugin_dir"
echo "Installed canon skill to $skill_dir/SKILL.md"
