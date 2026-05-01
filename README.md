# canon

[![CI](https://github.com/progbeat/canon/actions/workflows/ci.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`canon` is a Codex plugin/skill for preserving thread-scoped decisions and
invariants during coding work.

The repo contains:

- a Codex plugin manifest: `.codex-plugin/plugin.json`
- a Codex skill: `skills/canon/SKILL.md`
- a small Rust CLI runtime: `canon`

The skill teaches agents to read accepted context before editing files, search
existing canon during exploration, and append durable decisions after they are
accepted. The CLI is the storage/runtime layer the skill uses.

## Agent Protocol

- Before editing or reviewing a file, inspect `canon r <relative-path>`.
- For broad context, search existing canon with `canon rg <term>`.
- After accepted decisions, record them with `canon a <relative-path> "<decision>"`.
- For general thread decisions, use `.` as the key.

Use repository-relative paths as keys because they are shorter and stable across
machines.

## Storage

V1 is Codex-first: canon is scoped by `CODEX_THREAD_ID`.

```text
<codex-session-file>.canon/
```

When the Codex session file cannot be found, `canon` falls back to:

```text
${CANON_HOME:-~/.canon}/codex/<CODEX_THREAD_ID>/
```

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/progbeat/canon/main/scripts/install.sh | bash
```

This installs the runtime and registers the repository as a local Codex plugin:

- the `canon` CLI to `~/.local/bin/canon`
- the plugin source checkout to `~/.codex/plugins/canon-source`
- a local plugin entry in `~/.agents/plugins/marketplace.json`
- a seeded Codex plugin cache under `~/.codex/plugins/cache/codex-plugins/canon`

Restart Codex, open Plugins > Local Plugins, then install `canon`.

For local runtime development from a checkout:

```sh
cargo install --path . --root ~/.local --force
```

For local plugin development from a checkout:

```sh
bash scripts/install.sh --local
```

## CLI Runtime

Print the current canon root:

```sh
canon
canon pwd
```

Get or create the canon file path for a key:

```sh
canon p src/lib.rs
canon path src/lib.rs
```

Read, write, append, delete:

```sh
canon r src/lib.rs
canon w src/lib.rs "Current known constraints."
canon a src/lib.rs "Keep validation order."
canon d src/lib.rs
```

`write` and `append` also read from stdin when text is omitted:

```sh
printf '%s\n' "Keep validation order." | canon a src/lib.rs
```

Search canon using ripgrep:

```sh
canon rg validation
canon rg validation -n
```

Long aliases are also available: `path`, `read`, `write`, `append`, `delete`,
`del`, and `rm`. `canon g` remains available as a short alias for `canon rg`.

## Environment

- `CODEX_THREAD_ID` is required in v1.
- `CANON_HOME` optionally overrides the base directory and disables Codex
  session-file sidecar discovery.
