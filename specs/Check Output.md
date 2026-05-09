# Check Output

`canon check` writes human-readable check output to stdout.

For each passing expectation, stdout contains exactly one line:

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

After all selected expectation results, stdout contains exactly one token usage
line:

```text
Token usage: total=<n> input=<n> (+ <n> cached) output=<n> (reasoning <n>)
```

Warnings, model fallback notices, timestamps, hashes, and internal diagnostics
are recorded in [[Logs]] rather than stdout.

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
Token usage: total=170,522 input=166,088 (+ 132,352 cached) output=4,434 (reasoning 3,723)
```
