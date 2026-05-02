---
name: canon
description: Use for coding edits/reviews to preserve task goals, accepted decisions, and constraints across context compaction.
---

# canon

Use `canon` as a thread-scoped canon for coding work. It preserves the original
goal, accepted decisions, constraints, invariants, and pitfalls that must keep
guiding the agent after context compaction.

## Before File Changes

Before editing, reviewing, or refactoring a file:

1. Identify the files likely to be touched.
2. Use repository-relative paths as canon keys; they are shorter and stable
   across machines.
3. Run `canon r <relative-path>` for each file.
4. If canon conflicts with the user request or current code, surface the conflict
   before editing.

Do this before applying patches or running formatters that rewrite files.

## Searching Canon

For broad tasks, feature work, or cross-file refactors, search existing canon:

```sh
canon rg <term>
```

Use `canon p <key>` only when you need the raw markdown path for normal file
tools.

## Recording Decisions

After a task goal, constraint, invariant, pitfall, or accepted decision is
established, append a short canon entry:

```sh
canon a <relative-path> "<decision>"
```

For multiline entries or text that is awkward to quote, pipe stdin:

```sh
printf '%s\n' "<decision>" | canon a <relative-path>
```

Use repository-relative file paths as keys for file-specific canon. Use `.` for
the original goal and general task constraints that do not belong to one file.

Record only behavior-shaping task context:

- user decisions
- original task goals
- invariants and constraints
- non-obvious tradeoffs
- pitfalls that could cause a wrong later edit

Do not record routine command output, obvious facts, transient TODOs, scratch
reasoning, or durable project specifications that belong in the repository.
