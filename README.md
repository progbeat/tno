# canon

[![CI](https://github.com/progbeat/canon/actions/workflows/ci.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Codex plugin for preserving accepted decisions, constraints, and goals when
long coding threads are compacted.

`canon` gives agents a thread-scoped canon of the original goal, accepted
decisions, constraints, invariants, and pitfalls they must keep honoring after
compaction. It is not project documentation; durable specifications belong in
the repository.

---

## Problem

Long Codex threads eventually get compacted. After compaction, an agent may lose
the reasoning that led to earlier decisions: the original goal becomes fuzzy,
accepted constraints disappear, and a later edit can accidentally undo a correct
fix made earlier in the same thread.

`canon` keeps those behavior-shaping anchors outside the compressed
conversation, but still scoped to the current thread. Before changing a file,
the agent reads the relevant canon entries; after an accepted decision, it
records the anchor that future edits must keep honoring.

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
| Record the task goal or a general constraint | `canon a . "<decision>"` |

Use repository-relative file paths as keys. Use `.` for the original goal and
general task constraints.

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

V1 is Codex-first: canon is scoped by `CODEX_THREAD_ID` and temp-backed by
default.

```text
${TMPDIR:-/tmp}/canon/codex/<CODEX_THREAD_ID>/
```

Use `CANON_HOME` when longer-lived storage is explicitly needed:

```text
${CANON_HOME}/codex/<CODEX_THREAD_ID>/
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
