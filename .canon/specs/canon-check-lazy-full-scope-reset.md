# `canon check` Lazy Full-Scope Reset

Let `project_size_tokens` estimate the number of tokens in the project's
Git-staged text content, excluding ignored content.

At the end of a `canon check` invocation, when final token usage data is
available, the following lazy full-scope reset policy is applied to
non-selected expectations:

```
def stochastic_round(x):
    n = floor(x)
    p = x - n
    return n + int(random() < p)

def lazy_full_scope_reset(final_total_tokens, project_size_tokens, non_selected_expectations):
    candidates = [e for e in non_selected_expectations if e.scope != ["."]]
    num_to_reset = min(
        stochastic_round(0.1 * final_total_tokens / project_size_tokens),
        len(candidates)
    )
    expectations_to_reset = random.sample(candidates, num_to_reset)
    for expectation in expectations_to_reset:
        set_scope(expectation, ["."])
    # Takes effect starting with the next `canon check` invocation.
```

This prevents long-lived narrowed scopes from missing rare cases where changes
outside the last known expectation scope could affect the expectation's answer.
