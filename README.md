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

To create project checks and run the fast `canon gate` before commits:

```sh
canon init
canon hook install
```

`canon init` creates `.canon/check.yml` from the embedded default template and
fails if that file already exists. `canon hook install` installs or reuses
`.git/hooks/pre-commit` and sets the local Git `core.hooksPath` to `.git/hooks`.
The pre-commit hook runs `canon gate`, a fast cache-only check that asks you to
run `canon check` when the staged snapshot has not been checked yet.

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
canon gate
canon check --fail-fast
canon check --ignore-cache
canon check --config other-check.yml
canon check -c other-check.yml
canon check 4 5 42
```

Create `.canon/check.yml`, install the pre-commit hook, run every project
expectation, validate cached results without asking the evaluator, stop after
the first failed expectation result, ignore reusable cache records, run checks
from a custom YAML config, or rerun selected 1-based expectations. `canon check`
is a project-facing AI expectation linter: it asks the configured evaluator
agent to answer each expectation from allowed files in the staged Git snapshot,
hides expected answers from it, and fails when observed answers do not exactly
match.

Long aliases: `path`, `read`, `write`, `append`, `delete`, `del`, and `rm`.

---

## Expectation Checks

`canon init` creates a default `.canon/check.yml`:

```yaml
version: 1
agent:
  model:
    primary: gpt-5.4-mini
    fallbacks:
      - gpt-5.3-codex-spark
  instructions: |
    Use the following response policy:
    Answer exactly `yes` or `no` only when the question asks for a yes/no answer and there is sufficient evidence to support the answer.
    If the question asks you to choose from lettered options, answer exactly with the option letter only, such as `a` or `b`.
    Do not guess.
    If there is not enough evidence to answer a valid question, answer exactly `idk`.
    If the question is malformed, answer exactly `malformed`.
    Scoring policy:
    * Correct answer: +5
    * Incorrect answer: -5
    * `idk`: 0
    * `malformed`: N/A, human review required
    Scope policy:
    * `scope` is the smallest allowed project context sufficient to answer the expectation with the same answer.
    * `scope` is not the list of evidence citations; cite supporting files or code inside the `evidence` response field.
    * Use `["."]` for project-wide absence, consistency, duplication, garbage, or overall quality unless a narrower scope can rule out relevant evidence outside it.
    * Propose a narrower scope of at most 4 paths only when the same answer remains fully supported inside it.
    * Successful scope narrowing: +1
    * Failed scope narrowing: -5
  ignore:
    - "target/**"
  plugins: []

expectations:
  - q: "Does this project satisfy the expectation described here?"
    a: "yes"
```

The single configured agent answers every selected expectation. `model` selects
the evaluator model. `ignore` lists repository-relative files or globs that the
evaluator must not read, and `.canon/**` plus `.git/canon/**` are always added
to the effective ignore list. `plugins` lists Codex plugin config keys such as
`canon@codex-plugins`; when the list is empty, `canon check` starts
`codex app-server` with plugin loading disabled. Expected answers are
single-line strings compared by exact equality; `idk` is just an ordinary answer
string.

`canon check` supplies the evaluator response protocol through thread developer
instructions, asks one question at a time as a JSON object with `scope` and
`question`, restricts ignored paths and narrowed scopes through Codex filesystem
permissions, and prints one JSON object per selected expectation to stdout as
soon as that result is available. The stdout object includes `timestamp`,
`number`, `result`, `prompt`, `expected`, `observed`, `evidence`, `scope`, and
`scopeHash`. `evidence` cites supporting files or code; `scope` is the smallest
allowed project context sufficient to answer with the same result, not a list of
evidence citations.

Per-expectation reusable results are stored in the Git directory under
`canon/cache/<ID>/history.jsonl`, where `ID` is a 120-bit base64url hash of the
expectation prompt and expected answer. `scopeHash` is a 120-bit base64url hash
of the staged Git contents visible through the record's scope. `canon check`
reuses matching cached pass and fail records, unless `--ignore-cache` is set.

Global diagnostic interrogation records are appended to
`git rev-parse --git-path canon/logs/0.jsonl`. At the start of `canon check`,
`0.jsonl` rotates only when it exceeds 128 KiB: `3.jsonl` is removed, `2` moves
to `3`, `1` to `2`, and `0` to `1`. The next diagnostic write creates a fresh
`0.jsonl`.

`canon gate` is cache-only and side-effect-free. It passes when every
expectation either has a reusable cached `pass` record for the current staged
snapshot or has reusable cached `fail` records for both the current staged
snapshot and `HEAD`, which means the failure was already present before the
staged change. It asks you to run `canon check` when cache records are missing
and prints new cached failures when they are present.

If an evaluator answers `malformed`, `canon check` retries once. If the final
answer is still `malformed`, the expectation fails and `canon check` prints a
human-review warning so a person can fix the expectation or prompt.

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
skills/canon/SKILL.md
scripts/install.sh
src/main.rs
```

The Codex skill defines the agent behavior. The Rust CLI is the storage/runtime
layer used by the skill.

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
