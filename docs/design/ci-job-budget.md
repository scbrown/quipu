# CI job budget candidate

The pinned CI workflow expands to 19 jobs on a pull request. This candidate
expands to 17: the shacl runner also executes the unchanged shacl+owl test
configuration, and the source-size runner executes the unchanged shape invariant
self-test and check. No feature configuration or command is dropped. The six
required branch-protection names remain unchanged. Extended correctness still
requires the combined invariant job. Main retains both invariant checks.

This is a modeled job-count change, not measured hosted runtime reduction. Review
must verify the serialized legs and observe a real post-adoption run. Runtime,
cache cost and queue fairness are not claimed unchanged. Docs and Conformance
already have PR path filters; their unchanged path lists include only their
relevant source/book/build inputs. A source change still needs Conformance. An
untouched docs or conformance path does not trigger that workflow.

Further path filtering inside required CI needs conditional-skip policy and
branch-protection review: skipping a required check must not strand the PR or
turn unknown coverage into green. This prototype deliberately does not remove a
required check. Publication follows the designated serial order and current source review, CI
and merge-helper gates.

The installed CD manifest was also checked: its four required names (`Build`,
`Test (shacl)`, `Test (default)`, `Pre-commit checks`) remain unchanged. The shape
and size checks are combined, never path-skipped; a failing component keeps the
combined job red. This candidate awaits review and observed
hosted fanout after adoption.

Hosted before snapshot for the current workflow-configuration PR485 materializes
16 CI jobs, plus one Docs, one Changelog scrub and three Conformance jobs (21
total). This is an in-progress snapshot: downstream aggregate jobs are not yet
materialized, so it is not a final workflow fanout count. The pinned YAML model
originally counted 18 CI jobs before and 16 after, including both aggregate jobs. Current main adds the query-performance job, yielding 19 before and 17 after.
The hosted after measurement must wait for publication and a completed natural run. The
snapshot and model must not be conflated.
