# canon

`canon` (`Thread Canon`) is a tiny CLI for thread-scoped decisions and invariants.

It is Codex-first in v1: canon is scoped by `CODEX_THREAD_ID`.

```text
<codex-session-file>.canon/
```

When the Codex session file cannot be found, `canon` falls back to:

```text
${CANON_HOME:-~/.canon}/codex/<CODEX_THREAD_ID>/
```

## Install

```sh
cargo install --path . --root ~/.local --force
```

This installs `canon` to `~/.local/bin/canon`.

## Usage

Print the current thread canon root:

```sh
canon
canon -r
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

## Codex Guidance

- Before editing or reviewing a file, inspect `canon r <file>` when present.
- For broad context, use `canon rg <term>`.
- After user-confirmed decisions, use `canon a <file> "<decision>"`.
- For cross-file decisions, use keys such as `decision:<topic>`.

## Environment

- `CODEX_THREAD_ID` is required in v1.
- `CANON_HOME` optionally overrides the base directory and disables Codex
  session-file sidecar discovery.

Future versions may add `CANON_SESSION_ID` or other provider-specific session
sources without changing the canon file format.
