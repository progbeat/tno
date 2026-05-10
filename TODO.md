# TODO

- Add a judge agent for scope-narrowing conflicts. When a narrowed-scope
  verification produces a different answer, the judge should receive both
  interrogation results, their evidence, and their scopes, then either resolve
  the final answer or require human review.
- Add an optional `--scope` / `-s` flag for `canon check -q` so ad-hoc queries
  can be run under a narrower enforced scope.
- Remove the concrete rolling-log file count and log size limits from the spec
  only, then change the implementation defaults from 128 KiB and 4 files to
  1 MiB and 8 files.
- After a model fallback happens during one `canon check`, keep using the
  fallback model for the rest of that run instead of retrying the primary model;
  primary-model usage limits refresh over hours, while a check run usually
  lasts minutes.
- Do not make repository changes while a `canon check` is currently running,
  because it uses stash/restore internally and restoring the stash can create
  conflicts.
