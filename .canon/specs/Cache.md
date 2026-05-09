# Cache

`CACHE_DIR` is the path returned by `git rev-parse --git-path canon/cache`.

`canon check` stores reusable per-expectation results under `CACHE_DIR`.

Each expectation has an `ID`. The `ID` is a 120-bit base64url string without
padding, encoded as exactly 20 characters, and derived from the expectation
question and expected answer.

Each expectation cache directory is `CACHE_DIR/ID`.

Each expectation cache directory stores reusable history in `history.jsonl`.
History files use JSON Lines format. Each non-empty line is one complete JSON
object.

Each history record contains at least these fields in order:

```text
timestamp
number
result
prompt
expected
observed
evidence
scope
scopeHash
```

`result` is either `pass` or `fail`.

`observed` is the evaluator answer that is compared with `expected`. History
records are written only for correct or incorrect answers. Non-answer states
such as `idk` and `malformed` are not written to history.

Reusable history consists only of verified answer records. `idk`, `malformed`,
and unparseable evaluator responses are not reusable history records.

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

Reusable cached pass and fail records are both valid cache hits.

`canon check --ignore-cache` forces evaluator interrogation even when reusable
exact-cache history records or cooldown-eligible history records exist.

If the evaluator returns a narrower scope with the same answer, `canon check`
verifies that strict-subset scope with an independent interrogation on that
restricted scope. The narrowed scope is reusable only when the observed answer
is unchanged. Failed narrowing attempts that change the answer are not written
to history.

The `canon gate` command is cache-only. It passes only when every selected
expectation has a reusable cached pass result for the current staged Git tree, a
fresh cooldown pass, or reusable cached fail results for both the current staged
Git tree and the `HEAD` tree.

When `canon gate` fails because reusable cache records are missing, it prints an
actionable message asking the user to run `canon check`. When `canon gate` fails
because there are new cached failures, it reports those failures and does not
suggest rerunning `canon check` as the fix.
