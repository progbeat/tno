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

To create project checks and run `canon check` automatically before commits:

```sh
canon init
canon hook install
```

`canon init` creates `.canon/check.yml` from the embedded default template and
fails if that file already exists. `canon hook install` installs or reuses
`.githooks/pre-commit` and sets the local Git `core.hooksPath` to `.githooks`.
Projects that keep check logs out of Git should ignore `.canon/logs/`.

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

```sh
canon init
canon hook install
canon check
canon check --fail-fast
canon check 4 5 42
```

Create `.canon/check.yml`, install the pre-commit hook, run every project
expectation, stop after the first failed expectation-agent result, or rerun
selected 1-based expectations. `canon check` is a project-facing AI expectation
linter: it asks configured evaluator agents to answer each expectation from
allowed files, hides expected answers from them, and fails when observed answers
do not exactly match.

Long aliases: `path`, `read`, `write`, `append`, `delete`, `del`, and `rm`.

---

## Expectation Checks

`canon init` creates a default `.canon/check.yml`:

```yaml
version: 1
instructions: |
  Use the following response policy:
  Answer exactly `yes` or `no` only when there is sufficient evidence to support the answer.
  Do not guess.
  If there is not enough evidence to answer `yes` or `no`, answer exactly `skip`.
  If question is malformed, answer exactly `malformed`.

  Scoring policy:
  * Correct answer: `+1`
  * Incorrect answer: `-1`
  * Skip: `0`

agents:
  project:
    ignore:
      - "target/**"
    plugins: []

expectations:
  - q: "Does this project satisfy the expectation described here?"
    a: "yes"
```

Each configured agent answers every selected expectation. `ignore` lists
repository-relative files or globs that the evaluator must not read, and
`.canon/**` is always added to the effective ignore list. `plugins` lists Codex
plugin config keys such as `canon@codex-plugins`; when every list is empty,
`canon check` starts `codex app-server` with plugin loading disabled. Expected
answers are single-line strings compared by exact equality. `skip` is incorrect
unless the expected answer is exactly `skip`.

`canon check` supplies the runtime response format, asks one expectation at a
time, reuses the same Codex app-server session per agent, restricts ignored
paths through Codex filesystem permissions, and reports the expectation number,
agent name, prompt, expected answer, observed answer, evidence, and rerun
command. Each result starts with a summary line in the form `<number>. OK` or
`<number>. FAIL`. By default, all selected expectation-agent results are checked
and reported; `--fail-fast` stops after the first failed result.

Each run also stores the interrogation report in
`.canon/logs/YYYYMMDD-HHMMSS.jsonl`. Every non-empty line is one JSON object for
one expectation-agent result with `timestamp`, `number`, `result`, `agent`,
`prompt`, `expected`, `observed`, and `evidence`. `result` is exactly `pass` or
`fail`, and timestamps are UTC. Records are appended and flushed as each
expectation-agent check finishes, so the current log can be watched with
`tail -f`. After writing a log, `canon check` removes old `.jsonl` files until
at most 10 logs remain and their total size is at most 100MB, unless the newest
log alone exceeds that size.

If an evaluator answers `malformed`, `canon check` fails that expectation and
prints a human-review warning so a person can fix the expectation or prompt.

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
