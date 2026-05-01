---
name: tno-thread-notes
description: Use for coding edits/reviews to read file notes before changes and record durable decisions with tno.
---

# Thread Notes

Use `tno` as thread-scoped margin notes for coding work.

## Before File Changes

Before editing, reviewing, or refactoring a file:

1. Identify the files likely to be touched.
2. Run `tno r <path>` for each file.
3. Treat "note not found" as no prior constraints.
4. If notes conflict with the user request or current code, surface the conflict
   before editing.

Do this before applying patches or running formatters that rewrite files.
If `tno` is unavailable or fails unexpectedly, say so briefly and continue
without notes unless the user asks you to stop.

## Searching Notes

For broad tasks, feature work, or cross-file refactors, search existing notes:

```sh
tno rg <term>
```

Use `tno p <key>` only when you need the raw markdown path for normal file
tools.

## Recording Decisions

After a durable decision is accepted or discovered, append a short note:

```sh
tno a <file> "<decision>"
```

Use file paths as keys for file-specific notes. Use topic keys such as
`decision:<topic>` for cross-file decisions.

Record only durable context:

- user decisions
- invariants and constraints
- non-obvious tradeoffs
- pitfalls that could cause a wrong future edit

Do not record routine command output, obvious facts, transient TODOs, or
scratch reasoning.
