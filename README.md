# canon

[![CI](https://github.com/progbeat/canon/actions/workflows/ci.yml/badge.svg)](https://github.com/progbeat/canon/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

AI agents can move fast, but they can also blur the original goal. The painful part is not just that they may rewrite working code or adjust tests, but that everything can still look “green” while the original intent is no longer protected, turning development into chasing regressions instead of validating progress.

With `canon`, the human writes down what must stay true. Then the agent can be
asked to fix the project and commit. Chasing the regressions becomes the
agent's job.

That is how this project was built: no human-written implementation code,
just Codex working against `canon` until the repo satisfied its own canon.

See the canon for canon in [`.canon/check.yml`](.canon/check.yml).
For terminology used by the CLI and docs, see the [Glossary](docs/GLOSSARY.md).

## Install

Requires Git, Rust/Cargo, and the Codex CLI.

```sh
cargo install --git https://github.com/progbeat/canon
```

Cargo is the recommended install path on macOS, Linux, and Windows. Release
binaries are planned.

## Quick Start

Initialize canon in your project's Git repository:

```sh
canon init
```

Edit `.canon/check.yml`, stage the project state you want checked, then run:

```sh
git add README.md src
canon check
```

After `canon check` has recorded passing expectations for the staged project,
install the pre-commit hook:

```sh
canon hook install
```

`canon hook install` installs a pre-commit hook that runs `canon gate`, blocking
staged changes that turn a previously passing canon expectation into a failing
one.

## Workflow

Do not try to write the whole canon upfront. Grow it from real misses.

1. Notice that something important is missing, regressed, or almost slipped
   through. For example, there is no Undo button, but the project should have
   one.
2. Add the expectation to `.canon/check.yml` with the answer the project should
   satisfy:

   ```yaml
   expectations:
     - q: "Is there an Undo button?"
       a: "yes"
   ```

3. Ask the agent to fix the project and commit.

The agent runs `canon check` while working. When a check fails, `canon check`
prints the observed answer and evidence, so the next fix has a concrete target
instead of just a red light.

> [!WARNING]
> `canon check` evaluates staged Git-tracked content from a temporary snapshot.
> Unstaged and untracked working tree files are not part of that evaluator
> snapshot.

You can also add a generator item to `.canon/check.yml` so `canon check`
expands Markdown specs under `.canon/specs` into additional expectations.

Keep `.canon/**` policy changes separate from implementation changes.
`canon gate` rejects staged changes that mix them.

For commits that clearly cannot affect canon expectations, such as a TODO or
`.gitignore` change, bypass the hook deliberately:

```sh
git commit -n -m "Update TODO"
```

## How It Scales

Each expectation is checked in a sandboxed scope. When a question can be
answered from a smaller part of the repository, `canon` narrows and verifies
that scope. The scope is enforced with filesystem permissions, so the evaluator
cannot read project files outside the allowed scope.

That keeps larger canons practical: checks untouched by the staged change can
be skipped, while broad expectations can still use the full project when they
need it. `cooldown` gives broad review expectations, such as dead files, dirty
hacks, or idiomaticity, their own review cadence: after a recent pass, they do
not have to be re-proven for every small commit.

## Commands

```sh
canon init
```

Create `.canon/check.yml`.

```sh
canon check
```

Run all expectations from `.canon/check.yml`.

```sh
canon check a7F K9m
```

Run selected expectations by unique ID prefix. Full expectation IDs are also
accepted.

```sh
canon check -q "Does the app still have Undo?"
```

Ask one uncached ad-hoc question.

```sh
canon check --ignore-cache
canon check --all
canon check --config other-check.yml
canon check -c other-check.yml
```

Force fresh evaluation, continue after a failed expectation, or use another config.
By default, `canon check` stops after the first final non-pass result.

```sh
canon gate
canon gate a7F K9m
```

Run the pre-commit gate manually for all or selected expectations.

```sh
canon hook install
```

Install the local pre-commit hook.

## License

MIT
