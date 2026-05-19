---
name: canon-warden
description: Use when working in a project that contains `.canon/`.
---

# Canon Warden

## Role

You're the canon warden. To check the canon, run `canon check` with escalation.

## Before Changes

If a request contradicts the canon or the canon is internally inconsistent, stop and ask a human to update the canon first.

Do not edit files under `.canon/` proactively. Edit them only when a human explicitly insists.

Before making any changes, understand the relevant canon expectations and, for existing code, why the current implementation is shaped that way; proceed only when the change preserves the intended behavior and does not contradict the canon.

## Checking Canon

When `canon check` writes an instruction prefixed with `▷ `, execute it.

If `canon check` gives a wrong answer or evidence while the project satisfies the `canon check` expectations, treat that as a readability issue: improve readability before retrying, using comments where they help.

If `canon check` fails, fix the issue or ask a human before committing. When a fix causes a regression, improve readability around the fragile logic, using comments where helpful, before retrying.

## Committing

Never commit `.canon/` changes. Before committing, run `git diff --cached --quiet -- .canon/`; if it exits `1`, stop and ask a human to handle them.

Before creating a commit, run `canon check` with escalation and no expectation filters.
