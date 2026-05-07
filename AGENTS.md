# AGENTS.md

Before making any edits to the project, run `git status --porcelain -- .canon/`.

If the command prints any output, make no changes anywhere in the project and ask a human to bring `.canon/` back to a clean git state.

If a request contradicts `.canon/check.yml`, stop and ask a human to update `.canon/check.yml` first.

Before every commit, run the full `canon check` command with no expectation-number filters and without `--fail-fast`.

If `canon check` fails, fix the issue or ask a human before committing.

Do not rely on a pre-commit hook to run `canon check`; this repository may keep the hook uninstalled while the installer remains available.

Never edit files under `.canon/`.

If you think something is wrong with `.canon/check.yml`, ask a human to fix it.
