# `canon gate` Command

`canon gate` is the fast pre-commit check for staged changes.

`canon gate` decides pass/fail using this logic:

```text
if any staged path is under .canon/**:
    if every staged path is under .canon/**:
        gate passes
    else:
        gate fails

has_missing = false

for each selected expectation:
    previous = cache result for that expectation at HEAD
    current = cache result for that expectation in the staged Git tree

    if previous is not cached fail and current is cached fail:
        gate fails

    if current is missing:
        has_missing = true

if has_missing:
    gate fails

gate passes
```

Every `canon gate` failure prints an actionable message.
