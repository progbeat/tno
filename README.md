# canon

[![CI](https://github.com/progbeat/canon/actions/workflows/ci.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Codex plugin for preserving thread-scoped decisions and invariants during
coding work.

`canon` gives agents a small session-scoped record they can read before edits,
search during exploration, and append to after durable decisions are accepted.

---

## Quick Start

```sh
curl -fsSL https://raw.githubusercontent.com/progbeat/canon/main/scripts/install.sh | bash
```

Restart Codex, open Plugins > Local Plugins, install `canon`, then use Codex normally.

The installer:

| Item | Location |
| --- | --- |
| CLI runtime | `~/.local/bin/canon` |
| Plugin source | `~/.codex/plugins/canon-source` |
| Local plugin registry | `~/.agents/plugins/marketplace.json` |
| Plugin cache | `~/.codex/plugins/cache/codex-plugins/canon` |

---

## What It Does

| Workflow | Agent action |
| --- | --- |
| Before editing a file | `canon r <relative-path>` |
| Search existing context | `canon rg <term>` |
| Record a file decision | `canon a <relative-path> "<decision>"` |
| Record a general decision | `canon a . "<decision>"` |

Use repository-relative file paths as keys. Use `.` for general thread decisions.

---

## CLI Reference

```sh
canon
canon pwd
```

Print the current canon root.

```sh
canon p src/lib.rs
canon path src/lib.rs
```

Get or create the markdown file path for a key.

```sh
canon r src/lib.rs
canon w src/lib.rs "Current known constraints."
canon a src/lib.rs "Keep validation order."
canon d src/lib.rs
```

Read, replace, append, and delete canon entries.

```sh
printf '%s\n' "Keep validation order." | canon a src/lib.rs
```

`write` and `append` read stdin when text is omitted.

```sh
canon rg validation
canon rg validation -n
```

Search canon with ripgrep. `canon g` is also available as a short alias for `canon rg`.

Long aliases: `path`, `read`, `write`, `append`, `delete`, `del`, and `rm`.

---

## Storage

V1 is Codex-first: canon is scoped by `CODEX_THREAD_ID`.

```text
<codex-session-file>.canon/
```

When the Codex session file cannot be found, `canon` falls back to:

```text
${CANON_HOME:-~/.canon}/codex/<CODEX_THREAD_ID>/
```

---

## Repository Layout

```text
.codex-plugin/plugin.json
skills/canon/SKILL.md
scripts/install.sh
src/main.rs
```

The Codex plugin and skill define the agent behavior. The Rust CLI is the
storage/runtime layer used by the skill.

---

## Development

Install the runtime from a checkout:

```sh
cargo install --path . --root ~/.local --force
```

Register the current checkout as the local plugin source:

```sh
bash scripts/install.sh --local
```

Run checks:

```sh
cargo fmt --check
cargo test
bash -n scripts/install.sh
```

## License

MIT
