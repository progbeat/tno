# `canon gate` Command

`canon gate` is the fast pre-commit check for staged changes.

`canon gate` decides pass/fail using the following logic:

```
def gate(active-expectations):
    if any staged path is under .canon/**:
        if every staged path is under .canon/**:
            return Pass
        else:
            return Fail
    has_missing = False
    for each expectation in active-expectations:
        prev-res = cached result for expectation at HEAD
        curr-res = cached result for expectation in the staged Git tree
        if prev-res is not Fail and curr-res is Fail:  # if regression:
            return Fail
        if curr-res is Missing:
            has_missing = True
    if has_missing:
        return Fail
    else:
        return Pass
```

Every `canon gate` failure prints an actionable message.
