# canon

[![CI](https://github.com/progbeat/canon/actions/workflows/ci.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`canon` is a Codex plugin and CLI for running AI expectation checks against the
staged Git snapshot of a project.

Use it to encode project-specific quality bars as questions in
`.canon/check.yml`, ask a configured Codex evaluator to answer those questions
from the staged code, and fail the check when the observed answers do not match
the expected answers.

---

## Why `canon check`

Traditional tests are best when the expected behavior can be executed directly.
Some repository expectations are harder to express that way:

- Is the README consistent with the current implementation?
- Does a refactor leave dead, duplicated, or obsolete files behind?
- Does the project still follow an architecture rule documented in a spec?
- Did the staged change introduce a maintainability issue that normal tests
  cannot see?

`canon check` treats those expectations as reviewable project policy. Each
expectation is a question, an exact expected answer, optional cooldown metadata,
and optional model reasoning effort. The expected answer is hidden from the
evaluator. The evaluator reads only the files it is allowed to read and returns
an observed answer with evidence and scope.

---

## Quick Start

Install the CLI and local Codex plugin:

```sh
curl -fsSL https://raw.githubusercontent.com/progbeat/canon/main/scripts/install.sh | bash
```

Restart Codex, open Plugins > Local Plugins, install `canon`, then initialize
checks in a repository:

```sh
canon init
```

Edit `.canon/check.yml` to define the expectations for the project, then run
the check against the staged snapshot:

```sh
git add README.md src
canon check
```

After `canon check` has populated reusable answers for the staged snapshot, add
the pre-commit hook:

```sh
canon hook install
```

The hook runs `canon gate`, a fast cache-only command. If the staged snapshot
has not been checked yet, `canon gate` asks you to run `canon check` before
committing.

The installer places files here:

| Item | Location |
| --- | --- |
| CLI runtime | `~/.local/bin/canon` |
| Plugin source | `~/.codex/plugins/canon-source` |
| Local plugin registry | `~/.agents/plugins/marketplace.json` |
| Plugin cache | `~/.codex/plugins/cache/codex-plugins/canon` |

---

## How `canon check` Works

`canon check` loads `.canon/check.yml`, expands any generated expectations, and
evaluates the staged Git snapshot by default. It temporarily exposes the index
contents at the real project root while preserving unstaged and untracked
working tree changes, then restores those changes after success or failure.

The config has one evaluator agent under the top-level `agent` section. The
agent defines developer instructions, model selection, reasoning effort, ignored
paths, and plugin loading. `.canon/**` and `.git/canon/**` are always denied to
evaluator threads, even though `canon check` itself parses the active config
before starting the evaluator.

For each selected expectation, the evaluator receives one question at a time
and must answer with a JSON object containing:

- `answer`: the observed answer, compared to `a` by exact string equality.
- `evidence`: supporting files or code.
- `scope`: the smallest allowed project context sufficient to answer the
  expectation correctly.

Expected answers are single-line strings. `yes`, `no`, `idk`, `malformed`, and
letter choices such as `a` or `b` are conventions established by the evaluator
instructions; the check result still uses exact string comparison.

If an evaluator reports a narrower scope, `canon check` verifies that narrowed
scope with an independent interrogation before accepting it. Narrow scopes make
future cache reuse cheaper because the reusable record is keyed by the visible
contents of that scope instead of the whole project.

Reusable results are stored under the Git directory in `canon/cache/`. A result
can be reused when the expectation prompt, expected answer, and scope hash still
match. `--ignore-cache` forces fresh evaluator interrogation. Cooldowns let a
recent passing result count as skipped until the configured duration expires.

After preflight, `canon gate` is cache-only and side-effect-free. It passes when
every selected expectation has a reusable cached pass, has a fresh cooldown
pass, or has a cached fail that was already present at `HEAD`. It fails quickly
when cache records are missing or when the staged snapshot introduces a new
cached failure. `.canon/**`-only staged changes pass without cache lookup when
the full-project `scopeHash` is unchanged from `HEAD`, because the visible
project content cannot regress.

`canon check` refuses staged changes that mix `.canon/**` paths with non-canon
paths. Keep policy changes separate from implementation changes so the
evaluator cannot be checked against a policy it is not allowed to read.

---

## Check Configuration

`canon init` creates `.canon/check.yml` from the embedded template in
[`templates/check.yml`](templates/check.yml) and refuses to overwrite an
existing config.

A compact example:

```yaml
version: 1

agent:
  model:
    primary: gpt-5.4-mini
    fallbacks:
      - gpt-5.3-codex-spark
  thinking: low
  instructions: |
    Answer exactly `yes` or `no` for yes/no questions.
    If there is not enough evidence, answer exactly `idk`.
    If the question is malformed, answer exactly `malformed`.
    Use `scope` for the smallest allowed project context sufficient to answer.
  ignore:
    - "target/**"
  plugins: []

expectations:
  - q: "Is README.md consistent with the current implementation?"
    a: "yes"
    cooldown: 4h

  - q: "Are there obsolete tracked files that can be removed safely?"
    a: "no"
    cooldown: 7d
    thinking: xhigh
```

Expectation items can also be generated from spec files:

```yaml
expectations:
  - path: "specs/*.md"
    q_template: |
      {content}
      ---
      Does the implementation fully satisfy this specification?
    a: "yes"
```

Generator paths are relative to the active config file directory. Matched files
are expanded lexicographically at the generator's position in the expectation
list and then use the same numbering, selection, cache, check, and gate behavior
as explicit `q`/`a` expectations.

The accepted schema and runtime behavior are implemented in the Rust code under
[`src/`](src/). The human-readable stdout contract for check results is
specified in `.canon/specs/Check Output.md`.

---

## CLI Reference

```sh
canon init
```

Create `.canon/check.yml` from the embedded default template. The command fails
without overwriting if the config already exists.

```sh
canon check
canon check 4 5 42
canon check --fail-fast
canon check --ignore-cache
canon check --config other-check.yml
canon check -c other-check.yml
```

Run all expectations, run selected 1-based expectation numbers, stop after the
first failed result, ignore reusable cache records, or use a custom YAML config.
With no expectation numbers, failures do not stop later expectations unless
`--fail-fast` is present.

```sh
canon check -q "Does README.md describe canon check?"
```

Ask one ad-hoc uncached question with the configured evaluator agent. `-q`
cannot be combined with expectation numbers, `--fail-fast`, or `--ignore-cache`.

```sh
canon gate
canon gate 4 5 42
```

Validate the selected expectations using only reusable cache and cooldown
records. This is the command run by the pre-commit hook.

```sh
canon hook install
```

Install or reuse `.git/hooks/pre-commit` and set the local Git
`core.hooksPath` to `.git/hooks`. If another hooks path or incompatible
pre-commit hook is already configured, the command refuses to overwrite it.

```sh
canon
canon pwd
```

Print the current thread-scoped canon root. This belongs to the experimental
note-taking workflow described at the end of this README.

---

## Runtime Data

Check cache and logs live inside the Git directory, not in tracked files.

Reusable expectation history is stored under:

```text
git rev-parse --git-path canon/cache/<ID>/history.jsonl
```

`ID` is a 120-bit base64url hash of the expectation prompt and expected answer.
Each reusable record includes a `scopeHash`, a 120-bit base64url hash of the
staged Git contents visible through that record's scope. History contains only
reusable pass and fail results. `idk`, `malformed`, and unparseable responses
require retry or human review and are not written as reusable history.

Runtime logs are written to:

```text
git rev-parse --git-path canon/logs/0.jsonl
```

`0.jsonl` rotates at the start of `canon check` only when it exceeds 128 KiB.
Logs include check start and finish events, expectation results, warnings,
model failures and fallbacks, token usage, agent communication, and evaluator
thread creation or reuse. Warnings and internal diagnostics go to the runtime
log rather than normal stdout/stderr.

Cache history files are compacted on an approximately 1-in-15 sample after
appends, keeping the latest reusable records. `canon check` also samples
cleanup of cache entries whose expectation IDs are no longer present in the
active config, so cache storage stays bounded in expectation under bounded
config and retained data.

---

## Repository Layout

```text
templates/check.yml
templates/pre-commit
src/main.rs
src/check.rs
src/check_config.rs
src/evaluator.rs
skills/canon/SKILL.md
scripts/install.sh
```

The Rust CLI implements check execution, staged snapshot handling, evaluator
orchestration, cache reuse, hook installation, and the experimental note
commands. The Codex skill describes how agents should use the experimental
thread-scoped canon workflow.

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

Run local checks:

```sh
cargo fmt --check
cargo test
bash -n scripts/install.sh
```

---

## Experimental: Thread-Scoped Canon

Before `canon check`, this project centered on a Codex note-taking workflow for
preserving accepted decisions, constraints, and goals when long coding threads
are compacted. That workflow still exists, but it is secondary to the
project-facing expectation checker.

The experimental workflow stores behavior-shaping anchors outside the compressed
conversation while keeping them scoped to the current Codex thread. Agents can
read relevant notes before editing and append new notes after a user accepts an
important decision.

Common commands:

| Workflow | Command |
| --- | --- |
| Read notes for a file | `canon r <relative-path>` |
| Search existing notes | `canon rg <term>` |
| Record a file decision | `canon a <relative-path> "<decision>"` |
| Record a task goal or general constraint | `canon a . "<decision>"` |

Use repository-relative file paths as keys. Use `.` for the original goal and
general task constraints.

Full note command reference:

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

Search thread-scoped notes with ripgrep. `canon g` is also available as a short
alias for `canon rg`.

Long aliases: `path`, `read`, `write`, `append`, `delete`, `del`, and `rm`.

V1 storage is Codex-first: notes are scoped by `CODEX_THREAD_ID` and temp-backed
by default.

```text
${TMPDIR:-/tmp}/canon/codex/<CODEX_THREAD_ID>/
```

Use `CANON_HOME` when longer-lived storage is explicitly needed:

```text
${CANON_HOME}/codex/<CODEX_THREAD_ID>/
```

## License

MIT
