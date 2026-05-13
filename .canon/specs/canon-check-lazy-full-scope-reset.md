# `canon check` Lazy Full-Scope Reset

Let `project_size_tokens` estimate the number of tokens in the project's
Git-staged content, excluding ignored content.

At the end of a `canon check` invocation, when final token usage data is
available, the following lazy full-scope reset policy is applied to
non-selected expectations:

```
def lazy_full_scope_reset(final_total_tokens, project_size_tokens, non_selected_expectations):
    num_to_reset_float = final_total_tokens / (10 * project_size_tokens)
    num_to_reset = floor(num_to_reset_float)
    # Stochastically round so E[num_to_reset] == num_to_reset_float.
    if random() < fractional_part(num_to_reset_float):
        num_to_reset += 1
    expectations_to_reset = random.sample(
        non_selected_expectations,
        min(num_to_reset, len(non_selected_expectations)),
    )
    for expectation in expectations_to_reset:
        reset_scope_and_invalidate_cache(expectation)
    # Takes effect starting with the next `canon check` invocation.
```

This prevents long-lived narrowed scopes from missing rare cases where changes
outside the last known expectation scope could affect the expectation's answer.
