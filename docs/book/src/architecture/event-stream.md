# Committed feed streams

`GET /events/stream` replays the existing event log. `GET /changes/stream`
replays complete transaction pages from the fact change feed, including named
graph writes. Both are read-only SSE endpoints. Changes use transaction IDs;
events use event offsets. Never submit a transaction ID to `/events/commit`.

Pass a nonnegative `since`, or `Last-Event-ID`. If both are supplied they must
agree. Each `quipu.events` or `quipu.changes` frame contains the corresponding
ordinary feed response as JSON; its SSE ID is the delivered page cursor.
Delivery does not acknowledge a consumer. Persist the applied data and cursor
atomically, and reconnect from that durable cursor after any failure.

SQLite commit hooks supply wake hints only. The stream crosses the writer
barrier and reads the committed feed before delivery; a rolled-back savepoint
or failed commit does not authorize an event. A watch channel coalesces hints;
read-after-subscribe prevents a missed commit during subscription. An hourly
backstop also checks for external writers. Fifteen-second heartbeat comments
perform no database reads.

There are at most 64 active streams and one transaction/event per read page.
Frames larger than 1 MiB terminate with an error event without acknowledging a
cursor. This is an output budget, not a bound on the memory needed to read one
large transaction. Consumers must refuse incomplete frames and retain the
last applied cursor on errors, with bounded reconnect backoff. Event filters
use `types` and `group`; transaction filters use `graph`. Combining unrelated
filter kinds is refused.

Streaming is a source capability. Enabling a consumer, replaying production
history, changing subscriptions or claiming scheduled acceptance requires its
separate operational decision and measurements.
