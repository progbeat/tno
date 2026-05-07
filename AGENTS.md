# AGENTS.md

If a request contradicts `.canon/check.yml`, stop and ask a human to update `.canon/check.yml` first.

Do not edit files under `.canon/` proactively. Edit them only when a human explicitly insists.

Never commit `.canon/` changes. Before committing, run `git diff --cached --quiet -- .canon/`; if it exits `1`, stop and ask a human to handle them.

Before every commit, run the full `canon check` command with no expectation-number filters and without `--fail-fast`.

If `canon check` fails, fix the issue or ask a human before committing.

If you think something is wrong with `.canon/check.yml`, ask a human to fix it.
