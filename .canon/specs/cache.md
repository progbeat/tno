# Cache

`CACHE_DIR` is `${CANON_STATE_DIR}/cache`.

`canon check` stores cached per-expectation results under `CACHE_DIR`.

Each expectation has an `ID`. The `ID` is a 120-bit base64url string without
padding, encoded as exactly 20 characters, and derived from the expectation
question and expected answer.

Each expectation cache directory is `CACHE_DIR/ID`.

Each expectation cache directory stores answer history in `history.jsonl`.
History files use JSON Lines format. Each non-empty line is one complete JSON
object.

Each history record contains at least these fields in order:

```text
timestamp
result
observed
evidence
scope
scopeHash
```

`result` is either `pass` or `fail`.

`observed` is the evaluator answer that is compared with the expected answer.
History records are written only for correct or incorrect answers. Non-answer
states such as `idk` and `malformed` are not written to history.

`timestamp` is UTC and records when the history record is produced.

`scope` is the scope for the cached result. It is either `["."]` or a
lexicographically sorted, duplicate-free list of normalized repository-relative
paths with redundant child paths removed when a parent directory path already
covers them.

`scopeHash` is a 120-bit base64url string without padding, encoded as exactly 20
characters. It is derived only from tracked staged Git contents under the
record's `scope` paths.

`canon check` compacts a history file with approximately a 1-in-15 chance after
appending a record. Compaction keeps at least the latest five valid JSON object
records.

When looking up a cached result, `canon check` scans `history.jsonl`
newest-to-oldest and selects the first record whose `scopeHash` matches the
current staged Git contents for that record's `scope`.
