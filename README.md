# canon

`canon` (`Thread Canon`) is a Codex plugin/skill for preserving
thread-scoped decisions and invariants during coding work.

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
- For named cross-file decisions, use keys such as `decision:<topic>`.

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

## Install Runtime

```sh
cargo install --path . --root ~/.local --force
```

This installs `canon` to `~/.local/bin/canon`.

For local skill development without plugin installation:

```sh
mkdir -p ~/.codex/skills/canon
cp skills/canon/SKILL.md ~/.codex/skills/canon/SKILL.md
```

## CLI Runtime

Print the current thread canon root:

```sh
canon
canon pwd
```

Get or create the canon file path for a key:

```sh
canon swap/src/swap/main.py
canon p swap/src/swap/main.py
```

Read, write, append, delete:

```sh
canon r swap/src/swap/main.py
canon w swap/src/swap/main.py "Current known constraints."
canon a swap/src/swap/main.py "Keep validation order."
canon d swap/src/swap/main.py
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
