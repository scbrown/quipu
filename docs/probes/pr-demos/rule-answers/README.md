# Explicit-support rule answers

The terminal recording compares CLI revision `f6739976ba3433ee8bb0530f55aa3fa209351e88` with clean source revision `bdb7709cd1cf5600485a87ebc648b9fc37b563fb`. Four previously rejected queries return supported, refuted, both and neither from independent evidence bits. Eight existing ASK controls pass in both runs. Each answer explicitly grants no action authority.

Replay with `scriptreplay --log-timing timing.log --log-out output.log` from this directory. This is a terminal recording, not an installation receipt. Both runs use isolated synthetic stores and leave the live graph untouched. Judgment admission and refusal controls are covered by the focused native tests.

Reproduce with `python3 docs/probes/pr-demos/rule-answers/demo.py <baseline-cli> <candidate-cli>` from the repository root. The script creates and removes private temporary stores and loads the source profile only into those stores.
