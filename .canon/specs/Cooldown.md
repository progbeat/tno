# Cooldown

An explicit expectation item may include an optional `cooldown` field next to
`q` and `a`:

```yaml
expectations:
  - q: "Does this expensive project-wide review still pass?"
    a: "yes"
    cooldown: 7d
```

`cooldown` values use compact positive duration syntax with exactly one integer
and one unit. Supported units are `s`, `m`, `h`, `d`, and `w`, for seconds,
minutes, hours, days, and weeks. Examples include `30m`, `4h`, `3d`, and `2w`.

Cooldown is a cache-only reuse path for expensive checks. It applies only when
the latest valid history record for that expectation has `result` equal to
`pass` and its `timestamp` is younger than the configured cooldown duration.

Cooldown reuse ignores `scopeHash`. A fresh cooldown pass may be reused even
when the current staged Git contents under the record's scope no longer match
the record's `scopeHash`.

Cooldown never applies to a latest `fail` result. A failed expectation must use
normal exact-cache behavior or be rechecked.

A cooldown hit does not append a history record, update the reused record, or
refresh the cooldown timestamp. Cooldown age is always measured from the
timestamp of that latest valid history record.

`canon check --ignore-cache` bypasses cooldown reuse and forces evaluator
interrogation.

`canon gate` is still cache-only and side-effect-free. A fresh cooldown pass is
accepted as a valid cached success for the selected expectation.

`canon check` skips reused passing results, including exact-cache pass hits and
passes reused through cooldown. A skipped expectation emits no per-expectation
stdout, contributes to the final skipped count instead of the passed count, and
satisfies the run unless another selected expectation fails.
