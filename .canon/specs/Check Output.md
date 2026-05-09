# Check Output

`canon check` writes human-readable check output to stdout.

For each passing expectation that is written to stdout, stdout contains exactly
one line:

```text
N. OK
```

For each failing expectation, stdout contains exactly six lines:

```text
N. FAILED
<escaped question>
Expected: <escaped expected>
Observed: <escaped observed>
Evidence: <escaped evidence>
Scope: <compact JSON array>
```

`N` is the 1-based expectation number from the active check configuration.

Embedded control characters in question, expected answer, observed answer, and
evidence are escaped before writing to stdout. Newline is rendered as `\n`.
Escaping prevents evaluator-provided text from injecting additional stdout
lines.

`Scope` is rendered as a compact JSON array on one line.

Reused passing results may be skipped when a reuse policy such as [[Cooldown]]
allows it. Skipped expectations emit no per-expectation stdout and count as
`skipped`, not `passed`, in the final summary. Failing results are never
skipped.

After all selected expectation results, stderr contains exactly one token usage
line:

```text
Token usage: total=<n> input=<n> (+ <n> cached) output=<n> (reasoning <n>)
```

The last line in stdout contains exactly one summary line:

```text
============================= <outcome-list> in <duration>s =============================
```

`outcome-list` is a comma-separated list of non-zero outcome counts using these
labels: `passed`, `failed` and `skipped`. If every count is zero, the
outcome list is `0 passed`.
The outcome text is surrounded by spaces and padded with `=` characters on both sides.

`passed` is the number of non-skipped selected expectations whose final result
is `pass`.
`failed` is the number of selected expectations whose final result is `fail`.
`skipped` is the number of selected expectations satisfied by a reused passing
result without evaluator interrogation or per-expectation stdout.

After a `canon check` run starts writing check output, stderr contains exactly
one token usage line:


If token usage data is unavailable, every numeric field is `0`.

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
Evidence: specs/Check Output.md requires human-readable stdout\nsrc/check.rs still writes render_check_log_record(record) to stdout
Scope: ["specs/Check Output.md","src/check.rs"]
========================= 1 passed, 1 failed in 0.42s =========================
```

Example stderr for the same check run:

```text
Token usage: total=170,522 input=166,088 (+ 132,352 cached) output=4,434 (reasoning 3,723)
```

Example summary with a skipped expectation:

```text
===================== 1 passed, 1 failed, 1 skipped in 0.42s =====================
```
