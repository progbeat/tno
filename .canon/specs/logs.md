# Logs

`LOGS_DIR` is `${CANON_STATE_DIR}/logs`.

`canon check` records runtime log events under `LOGS_DIR`.

Runtime logs are JSON Lines files. Each non-empty line is one complete JSON object.

The active runtime log file is `LOGS_DIR/0.jsonl`. Older runtime logs may be
retained as rotated files in the same directory.

Runtime log retention is bounded to a finite set of numeric files.

Before the first runtime log event is written during a `canon check` invocation,
`0.jsonl` is rotated when it is larger than the configured runtime log size
limit. Rotation deletes the oldest retained log when present, renames existing
numeric logs to the next older numeric slot, and renames `0.jsonl` to `1.jsonl`.
The next runtime log write creates a new `0.jsonl`.

Each log record is appended and flushed as soon as the record is produced.

Each runtime log event contains these fields:

```text
timestamp
level
event
```

`timestamp` is UTC and records when the event is produced.

`level` is a single-line severity label.

`event` is a single-line event name.

Additional fields depend on the event type. Event-specific data is recorded as
structured JSON fields, not encoded into a human-readable message string.

Runtime logs include events for check start, expectation results, warnings,
model failures, model fallback attempts, token usage, agent communication,
evaluator thread creation, evaluator thread reuse, review-required diagnostics,
and check finish.

Events may include a single-line `message` field only when human-readable text
adds useful context beyond the structured fields.

Agent communication events record the boundary between `canon check` and the
evaluator agent. The runtime log contains enough structured information to audit
tasks sent to the evaluator and raw assistant responses received before parsing
or repair.

Agent communication request payloads do not include hidden expected answers.

Agent communication logs should be useful for debugging evaluator exchanges. In
practice, they should prefer recording the raw request sent to the evaluator, the
raw response received from it, and enough nearby context to understand how that
exchange fits into the check run.

Runtime logs should also make evaluator thread creation and reuse understandable
when that behavior matters for debugging. The effective evaluator instructions
used for a thread should be inspectable from the logs.

Evaluator agent sessions do not have read access to `LOGS_DIR`.

Warnings, model fallback notices, malformed-answer notices, full-project `idk`
review-required diagnostics, timestamps, hashes, and internal diagnostics are
recorded in runtime logs.

This document defines the runtime log container format and required coverage.
Event-specific schemas are defined by the event types themselves; examples in
this document illustrate JSON Lines shape, not a complete event registry.

Hypothetical log event examples:

```json
{"timestamp":"2026-05-09T10:00:02Z","level":"warning","event":"model.fallback","from":"gpt-5.3-codex-spark","to":"gpt-5.4-mini","reason":"usageLimitExceeded"}
{"timestamp":"2026-05-09T10:00:08Z","level":"info","event":"token.usage","total":170522,"input":166088,"cached_input":132352,"output":4434,"reasoning_output":3723}
```
