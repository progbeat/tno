# TODO

- Add a judge agent for scope-narrowing conflicts. When a narrowed-scope
  verification produces a different answer, the judge should receive both
  interrogation results, their evidence, and their scopes, then either resolve
  the final answer or require human review.
- Add an optional `--scope` / `-s` flag for `canon check -q` so ad-hoc queries
  can be run under a narrower enforced scope.
