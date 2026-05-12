# `canon check` Output

`canon check` writes human-readable check output to stdout.

For each passing expectation that is written to stdout, stdout contains exactly
one line:

```text
N. OK
```

For each failed expectation, stdout contains exactly six lines:

```text
N. FAILED
<escaped question>
Expected: <escaped expected>
Observed: <escaped observed>
Evidence: <escaped evidence>
Scope: <compact JSON array>
```

For each errored expectation, stdout contains exactly five lines:

```text
N. ERROR
<escaped question>
Expected: <escaped expected>
Observed: <escaped observed>
Evidence: <escaped evidence>
```

`N` is the 1-based expectation number from the active check configuration.

Embedded control characters in question, expected answer, observed answer, and
evidence are escaped before writing to stdout. Newline is rendered as `\n`.
Escaping prevents evaluator-provided text from injecting additional stdout
lines.

`Scope` is rendered as a compact JSON array on one line.

Skipped expectations emit no per-expectation stdout. Failing results are never
skipped.

After all selected expectations have been processed, stderr contains exactly one
token usage line:

```text
Token usage: total=<n> input=<n> (+ <n> cached) output=<n> (reasoning <n>)
```

If token usage data is unavailable, every numeric field is `0`.

After the token usage line is written, stdout contains exactly one summary line.
This summary line is the last line written by `canon check`:

```text
============================= <outcome-list> in <duration>s =============================
```

`outcome-list` is a comma-separated list of non-zero outcome counts in this
order: failed, error/errors, passed, skipped. If every count is zero, the
outcome list is `0 passed`. The outcome text is surrounded by spaces and padded
with `=` characters on both sides.

Outcome labels follow pytest pluralization: `failed`, `passed`, and `skipped`
are used for both singular and plural counts; `error` is used for one error and
`errors` for every other error count.

`passed` is the number of non-skipped selected expectations whose final result
is `pass`.
`failed` is the number of selected expectations whose final result is `fail`
because the evaluator returned an answer that was parsed successfully and did
not match the expected answer.
`errors` is the number of selected expectations whose final result requires
human review.
`skipped` is the number of non-selected expectations.

Warnings, model fallback notices, review-required diagnostics, timestamps,
hashes, and internal diagnostics are recorded in [[Logs]] rather than stdout or
stderr.
Early command-line, configuration, and preflight errors may use normal CLI error
output when `canon check` has not started writing check output.

Example stdout for a check run with one passing expectation and one failing
expectation:

```text
1. OK
2. FAILED
Does `canon check` write stdout in the format specified by [[Check Output]]?
Expected: yes
Observed: no
Evidence: specs/check-output.md requires human-readable stdout\nsrc/check.rs still writes render_check_log_record(record) to stdout
Scope: ["specs/check-output.md","src/check.rs"]
========================= 1 failed, 1 passed in 0.42s =========================
```

Example stderr for the same check run, written after the selected expectation
results and before the stdout summary:

```text
Token usage: total=170,522 input=166,088 (+ 132,352 cached) output=4,434 (reasoning 3,723)
```

Example summary with a skipped expectation:

```text
===================== 1 failed, 1 passed, 1 skipped in 0.42s =====================
```

Example summary with an errored expectation:

```text
====================== 1 failed, 1 error, 1 passed in 0.42s ======================
```
