#!/usr/bin/env bash
set -euo pipefail

PLUGIN_NAME="canon"
REPO_URL="${CANON_REPO_URL:-https://github.com/progbeat/canon}"
SOURCE_DIR="${CANON_SOURCE_DIR:-$HOME/.codex/plugins/canon-source}"
MARKETPLACE_FILE="${CANON_MARKETPLACE_FILE:-$HOME/.agents/plugins/marketplace.json}"
CODEX_PLUGIN_CACHE_ROOT="${CANON_PLUGIN_CACHE_ROOT:-$HOME/.codex/plugins/cache/codex-plugins/canon}"
CARGO_ROOT="${CARGO_INSTALL_ROOT:-$HOME/.local}"

usage() {
  cat <<'EOF'
canon installer

Usage:
  bash scripts/install.sh            Clone/update and register as a local Codex plugin
  bash scripts/install.sh --local    Register the current checkout for local plugin development
  bash scripts/install.sh --help
EOF
}

ensure_clean_checkout() {
  local source_dir="$1"
  if [ -n "$(git -C "$source_dir" status --porcelain)" ]; then
    echo "canon installer: $source_dir has local changes" >&2
    echo "Commit or stash them first, or use --local from the checkout you want Codex to load." >&2
    exit 1
  fi
}

update_source_checkout() {
  local source_dir="$1"
  ensure_clean_checkout "$source_dir"
  git -C "$source_dir" fetch origin
  git -C "$source_dir" pull --ff-only origin main
}

install_runtime() {
  local plugin_path="$1"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "canon installer: cargo is required" >&2
    exit 1
  fi
  cargo install --path "$plugin_path" --root "$CARGO_ROOT" --force
}

register_marketplace() {
  local manifest_path="$1"
  local dest_path="$2"
  local plugin_source_path="$3"

  python3 - "$manifest_path" "$dest_path" "$plugin_source_path" <<'PY'
import json
import os
import sys

manifest_path, dest_path, plugin_source_path = sys.argv[1:4]
owner_name = os.environ.get("USER", "unknown")
marketplace_root = os.path.abspath(os.path.join(os.path.dirname(dest_path), "..", ".."))
plugin_source_abs = os.path.abspath(plugin_source_path)

with open(manifest_path) as f:
    manifest = json.load(f)

relative_plugin_path = os.path.relpath(plugin_source_abs, marketplace_root)
if relative_plugin_path == ".":
    marketplace_path = "./"
elif relative_plugin_path.startswith(".."):
    raise SystemExit(
        "canon installer: plugin source must live inside marketplace root: "
        + marketplace_root
    )
else:
    marketplace_path = "./" + relative_plugin_path.replace(os.sep, "/")

entry = {
    "name": manifest["name"],
    "description": manifest["description"],
    "version": manifest["version"],
    "author": manifest.get("author", {"name": owner_name}),
    "source": {
        "source": "local",
        "path": marketplace_path,
    },
    "policy": {
        "installation": "AVAILABLE",
        "authentication": "ON_INSTALL",
    },
    "category": manifest.get("interface", {}).get("category", "Productivity"),
}
if "interface" in manifest:
    entry["interface"] = manifest["interface"]

if os.path.exists(dest_path):
    with open(dest_path) as f:
        dest = json.load(f)
else:
    dest = {
        "name": "codex-plugins",
        "description": "Codex plugin marketplace",
        "owner": {"name": owner_name},
        "interface": {"displayName": "Local Plugins"},
        "plugins": [],
    }

dest.setdefault("name", "codex-plugins")
dest.setdefault("description", "Codex plugin marketplace")
dest.setdefault("owner", {"name": owner_name})
dest.setdefault("interface", {"displayName": "Local Plugins"})
dest.setdefault("plugins", [])

for index, plugin in enumerate(dest["plugins"]):
    if plugin and plugin.get("name") == manifest["name"]:
        dest["plugins"][index] = entry
        action = "Updated"
        break
else:
    dest["plugins"].append(entry)
    action = "Added"

os.makedirs(os.path.dirname(dest_path), exist_ok=True)
with open(dest_path, "w") as f:
    json.dump(dest, f, indent=2)
    f.write("\n")

print(f"{action} canon plugin entry in {dest_path}")
PY
}

refresh_plugin_cache() {
  local source_dir="$1"
  local refreshed=0

  if [ -d "$CODEX_PLUGIN_CACHE_ROOT" ]; then
    for cache_dir in "$CODEX_PLUGIN_CACHE_ROOT"/*/; do
      [ -d "$cache_dir" ] || continue
      echo "Refreshing Codex plugin cache at $cache_dir"
      rsync -a --delete --delete-excluded --exclude ".git" --exclude "target" "$source_dir/" "$cache_dir"
      refreshed=1
    done
  fi

  if [ "$refreshed" -eq 0 ]; then
    local seed_dir="$CODEX_PLUGIN_CACHE_ROOT/local"
    echo "Seeding Codex plugin cache at $seed_dir"
    mkdir -p "$seed_dir"
    rsync -a --delete --delete-excluded --exclude ".git" --exclude "target" "$source_dir/" "$seed_dir/"
  fi
}

LOCAL_MODE=false
for arg in "$@"; do
  case "$arg" in
    --local)
      LOCAL_MODE=true
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "canon installer: unknown argument: $arg" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [ "$LOCAL_MODE" = true ]; then
  PLUGIN_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  echo "Local mode: using $PLUGIN_PATH as plugin source"
else
  PLUGIN_PATH="$SOURCE_DIR"
  if [ -d "$SOURCE_DIR/.git" ]; then
    echo "Updating canon source checkout at $SOURCE_DIR"
    update_source_checkout "$SOURCE_DIR"
  else
    echo "Cloning canon to $SOURCE_DIR"
    mkdir -p "$(dirname "$SOURCE_DIR")"
    git clone "$REPO_URL" "$SOURCE_DIR"
  fi
fi

PLUGIN_MANIFEST="$PLUGIN_PATH/.codex-plugin/plugin.json"
if [ ! -f "$PLUGIN_MANIFEST" ]; then
  echo "canon installer: missing plugin manifest at $PLUGIN_MANIFEST" >&2
  exit 1
fi

install_runtime "$PLUGIN_PATH"
register_marketplace "$PLUGIN_MANIFEST" "$MARKETPLACE_FILE" "$PLUGIN_PATH"
refresh_plugin_cache "$PLUGIN_PATH"

cat <<EOF

canon installed.

Next steps:
  1. Restart Codex.
  2. Open Plugins > Local Plugins.
  3. Install canon.

Plugin source: $PLUGIN_PATH
Marketplace: $MARKETPLACE_FILE
Runtime: $CARGO_ROOT/bin/canon
EOF
