# Interrogation Policy

An interrogation is a `canon check` evaluator turn for one expectation under one
enforced scope.

The enforced scope is supplied in evaluator thread developer instructions.

Each evaluator task input is exactly the expectation question string.

Expected answers are not included in evaluator task inputs.

The evaluator response format is exactly one JSON object with these keys in this
order and no extra keys:

```text
answer
evidence
scope
```

`answer` is a single-line string.

`evidence` is a string citing supporting files or code. Evidence citations are
separate from scope.

`scope` is either `["."]` or a JSON array of normalized repository-relative path
strings.

`canon check` asks one expectation at a time through `codex app-server`.

Evaluator Codex threads are ephemeral to one `canon check` invocation.

Within one invocation, evaluator Codex threads are reused only for
interrogations with the same enforced scope. Interrogations with different
enforced scopes use separate evaluator Codex threads.

When interrogating without a reusable cache hit, `canon check` uses the scope
from the latest reusable history record for that expectation, or `["."]` if no
reusable history exists. This lookup is not filtered to passing records only.

When an interrogation with a restricted scope returns `idk`, `canon check`
retries with full project scope and does not treat the restricted `idk` as final
when full-scope evidence can answer.

When an interrogation with a full project scope returns `idk`, human review is
required and that response is not written to reusable history.

If the evaluator returns correct or incorrect answer and a narrower scope,
`canon check` verifies that strict-subset scope with an independent interrogation
on that restricted scope. The narrowed scope is accepted only when the observed
answer is unchanged. Failed narrowing attempts that change the answer are not reusable.

Evaluator-proposed scope cannot widen beyond the enforced scope. A proposed
scope that widens the enforced scope is rejected and is not written
to reusable history.

`canon check` uses `agent.model.primary` as the primary evaluator model.
Configured `agent.model.fallbacks` are tried in order only after technical
app-server or model failures such as `usageLimitExceeded`.

`canon check` requires human review when the evaluator answer is `malformed`.
