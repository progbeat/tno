# tno

`tno` (`Thread NOtes`) is a tiny CLI for thread-scoped markdown notes.

It is Codex-first in v1: notes are scoped by `CODEX_THREAD_ID`.

```text
<codex-session-file>.tno/
```

When the Codex session file cannot be found, `tno` falls back to:

```text
${TNO_HOME:-~/.thread-notes}/codex/<CODEX_THREAD_ID>/
```

## Install

```sh
cargo install --path . --root ~/.local --force
```

This installs `tno` to `~/.local/bin/tno`.

## Usage

Print the current thread notes root:

```sh
tno
tno -r
```

Get or create the note path for a key:

```sh
tno swap/src/swap/main.py
tno p swap/src/swap/main.py
```

Read, write, append, delete:

```sh
tno r swap/src/swap/main.py
tno w swap/src/swap/main.py "Current known constraints."
tno a swap/src/swap/main.py "Keep validation order."
tno d swap/src/swap/main.py
```

Search notes using ripgrep:

```sh
tno g validation
tno rg validation -n
```

Long aliases are also available: `path`, `read`, `write`, `append`, `delete`,
`del`, `rm`, and `rg`.

## Codex Guidance

- Before editing or reviewing a file, inspect `tno r <file>` when present.
- For broad context, use `tno g <term>`.
- After user-confirmed decisions, use `tno a <file> "<decision>"`.
- For cross-file decisions, use keys such as `decision:<topic>`.

## Environment

- `CODEX_THREAD_ID` is required in v1.
- `TNO_HOME` optionally overrides the base directory and disables Codex
  session-file sidecar discovery.

Future versions may add `TNO_SESSION_ID` or other provider-specific session
sources without changing the note file format.
