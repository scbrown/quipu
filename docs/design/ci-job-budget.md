# Private CI job budget candidate

The pinned CI workflow expands to 18 jobs on a pull request. This candidate
expands to 16: the shacl runner also executes the unchanged shacl+owl test
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
required check. No public push: publication waits for PR189 landing and the
candidate's turn in the designated serial order.

The installed CD manifest was also checked: its four required names (`Build`,
`Test (shacl)`, `Test (default)`, `Pre-commit checks`) remain unchanged. The shape
and size checks are combined, never path-skipped; a failing component keeps the
combined job red. This remains a private candidate awaiting review and observed
hosted fanout after adoption.
