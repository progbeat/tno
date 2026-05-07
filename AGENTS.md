# AGENTS.md

Before making any edits to the project, run `git status --porcelain -- .canon/`.

If the command prints any output, make no changes anywhere in the project and ask a human to bring `.canon/` back to a clean git state.

If a request contradicts `.canon/check.yml`, stop and ask a human to update `.canon/check.yml` first.

Before every commit, run the full `canon check` command with no expectation-number filters and without `--fail-fast`.

If `canon check` fails, fix the issue or ask a human before committing.

Never edit files under `.canon/` unless a human explicitly asks you to edit those files.

If you think something is wrong with `.canon/check.yml`, ask a human to fix it.
