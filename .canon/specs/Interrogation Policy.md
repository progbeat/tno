# Interrogation Policy

An interrogation is a `canon check` evaluator turn for one expectation under one
enforced scope.

Each evaluator task input is exactly one JSON object with only these fields:

```text
scope
question
```

`scope` is the enforced scope for the interrogation.

`question` is the expectation question.

Expected answers are not included in evaluator task inputs.

The evaluator response format contains the observed answer, evidence, and
proposed scope. Evidence citations are separate from scope.

`canon check` asks one expectation at a time through `codex app-server`.

Evaluator Codex threads are ephemeral to one `canon check` invocation.

Within one invocation, evaluator Codex threads are reused only for
interrogations with the same enforced scope. Interrogations with different
enforced scopes use separate evaluator Codex threads.

When interrogating without a reusable cache hit, `canon check` uses the latest
history scope for that expectation, or `["."]` if no history exists.

When an interrogation with a restricted scope returns `idk`, `canon check`
retries with full project scope and does not treat the restricted `idk` as final
when full-scope evidence can answer.

When an interrogation with a full project scope returns `idk`, human review is required.

If the evaluator returns correct or incorrect answer and a narrower scope,
`canon check` verifies that strict-subset scope with an independent interrogation
on that restricted scope. The narrowed scope is accepted only when the observed
answer is unchanged. Failed narrowing attempts that change the answer are not reusable.

`canon check` uses `agent.model.primary` as the primary evaluator model.
Configured `agent.model.fallbacks` are tried in order only after technical
app-server or model failures such as `usageLimitExceeded`.

`canon check` requires human review when the evaluator answer is `malformed`.
