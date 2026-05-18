# AGENTS.md

If a request contradicts the `canon check` expectations or those expectations are internally inconsistent, stop and ask a human to update them first.

Do not edit files under `.canon/` proactively. Edit them only when a human explicitly insists.

Before making any changes, understand the relevant canon expectations and, for existing code, why the current implementation is shaped that way; proceed only when the change preserves the intended behavior and does not contradict the canon.

Treat tokens as a scarce resource. Avoid increasing token usage unless the correctness benefit justifies it, and prefer designs that preserve or reduce the amount of model work needed to answer canon questions correctly.

Never commit `.canon/` changes. Before committing, run `git diff --cached --quiet -- .canon/`; if it exits `1`, stop and ask a human to handle them.

Before creating a commit as an agent, run `canon check` with escalation and no expectation filters. This is separate from the installed Git pre-commit hook; the hook itself runs `canon gate`.

If `canon check` gives a wrong answer or evidence while the project satisfies the `canon check` expectations, treat that as a readability issue: improve readability before retrying, using comments where they help.

If `canon check` fails, fix the issue or ask a human before committing. When a fix causes a regression, improve readability around the fragile logic, using comments where helpful, before retrying.
