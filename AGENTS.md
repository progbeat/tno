# AGENTS.md

If a request contradicts the `canon check` expectations or those expectations are internally inconsistent, stop and ask a human to update them first.

Do not edit files under `.canon/` proactively. Edit them only when a human explicitly insists.

Never commit `.canon/` changes. Before committing, run `git diff --cached --quiet -- .canon/`; if it exits `1`, stop and ask a human to handle them.

Before every commit, run the full `canon check` command with no expectation-number filters and without `--fail-fast`.

If `canon check` gives a wrong answer or evidence while the project satisfies the `canon check` expectations, treat that as a readability issue: clarify the non-obvious logic with concise comments before retrying.

If `canon check` fails, fix the issue or ask a human before committing.
