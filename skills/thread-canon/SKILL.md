---
name: thread-canon
description: Use for coding edits/reviews to read canon before changes and record durable decisions/invariants.
---

# Thread Canon

Use `canon` as thread-scoped accepted decisions and invariants for coding work.

## Before File Changes

Before editing, reviewing, or refactoring a file:

1. Identify the files likely to be touched.
2. Use repository-relative paths as canon keys; they are shorter and stable
   across machines.
3. Run `canon r <relative-path>` for each file.
4. Treat "canon not found" as no prior constraints.
5. If canon conflicts with the user request or current code, surface the conflict
   before editing.

Do this before applying patches or running formatters that rewrite files.
If `canon` is unavailable or fails unexpectedly, say so briefly and continue
without canon unless the user asks you to stop.

## Searching Canon

For broad tasks, feature work, or cross-file refactors, search existing canon:

```sh
canon rg <term>
```

Use `canon p <key>` only when you need the raw markdown path for normal file
tools.

## Recording Decisions

After a durable decision is accepted or discovered, append a short canon entry:

```sh
canon a <relative-path> "<decision>"
```

Use repository-relative file paths as keys for file-specific canon. Use `.` for
general thread decisions that do not belong to one file. Use topic keys such as
`decision:<topic>` only when a named topic is clearer than `.`.

Record only durable context:

- user decisions
- invariants and constraints
- non-obvious tradeoffs
- pitfalls that could cause a wrong future edit

Do not record routine command output, obvious facts, transient TODOs, or
scratch reasoning.
