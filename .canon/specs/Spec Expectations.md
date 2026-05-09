# Spec Expectations

`canon check` can generate expectations from spec files.

The active check configuration's `expectations` list may contain explicit
expectation items and generator items.

An explicit expectation item contains `q` and `a`:

```yaml
expectations:
  - q: "Does this behavior work?"
    a: "yes"
```

A generator item contains `path`, `q_template`, and `a`:

```yaml
expectations:
  - path: "specs/*.md"
    q_template: |
      {content}
      ---
      Is this specification implemented?
    a: "yes"
```

`q` and `q_template` are mutually exclusive. `path` is allowed only on generator
items.

Each `path` is relative to the directory containing the active check
configuration file.

Each `q_template` must contain exactly one `{content}` placeholder. No other
`{...}` placeholders are allowed. For every matched spec file, `canon check`
renders `q_template` by substituting `{content}` with the UTF-8 file contents to
produce the generated expectation question.

Generated spec expectations use the generator item's `a` value as the expected
answer.

Matched spec files are expanded in lexicographic path order. Generated
expectations are inserted at the generator item's position in the
`expectations` list.

Generated spec expectations use the same numbering, selection, cache, `canon
check`, and `canon gate` behavior as explicit expectations.

Config loading fails when a generator item has an invalid `q_template`, a `path`
matches no files, a matched spec file is not valid UTF-8, or the same spec path
is expanded more than once.
