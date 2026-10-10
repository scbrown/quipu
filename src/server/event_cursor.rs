//! Private transport core: replay offsets are delivery positions, never ACKs.

const MAX_FRAME_BYTES: usize = 1024 * 1024;

pub(super) fn resume_offset(
    since: Option<i64>,
    last_id: Option<&str>,
) -> Result<i64, &'static str> {
    let header = last_id
        .map(|id| {
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Last-Event-ID must be a nonnegative decimal offset");
            }
            id.parse::<i64>()
                .map_err(|_| "Last-Event-ID exceeds offset range")
        })
        .transpose()?;
    if since.is_some_and(|offset| offset < 0) {
        return Err("since must be nonnegative");
    }
    if let (Some(query), Some(header)) = (since, header) {
        if query != header {
            return Err("since and Last-Event-ID disagree");
        }
    }
    Ok(since.or(header).unwrap_or(0))
}

/// Frames only rows reread from the committed log. Filtered offsets may have gaps.
/// The caller persists its own ACK only after applying the event successfully.
pub(super) fn validate_frame(previous: i64, offset: i64, json: &str) -> Result<(), &'static str> {
    if previous < 0 || offset <= previous {
        return Err("event offset must advance monotonically");
    }
    // The serving adapter must pass single-line serde_json serialization, not
    // untrusted preformatted SSE fields. Refuse rather than repair injection.
    if json.contains(['\r', '\n']) {
        return Err("event data must be single-line JSON");
    }
    if json.len() > MAX_FRAME_BYTES.saturating_sub(64) {
        return Err("event exceeds transport frame budget");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(previous: i64, offset: i64, json: &str) -> Result<String, &'static str> {
        validate_frame(previous, offset, json)?;
        Ok(format!(
            "id: {offset}\nevent: quipu.event\ndata: {json}\n\n"
        ))
    }

    #[test]
    fn reconnect_and_explicit_cursor_agree() {
        assert_eq!(resume_offset(None, None), Ok(0));
        assert_eq!(resume_offset(None, Some("42")), Ok(42));
        assert_eq!(resume_offset(Some(42), Some("42")), Ok(42));
        assert!(resume_offset(Some(41), Some("42")).is_err());
    }

    #[test]
    fn malformed_cursor_never_rewinds_to_genesis() {
        for id in ["", "-1", "+1", " 1", "1\n", "9223372036854775808"] {
            assert!(resume_offset(None, Some(id)).is_err(), "{id:?}");
        }
        assert!(resume_offset(Some(-1), None).is_err());
    }

    #[test]
    fn filtered_gaps_are_valid_but_replay_is_not_an_implicit_ack() {
        let cursor = 42;
        assert!(frame(cursor, 99, "{\"offset\":99}").is_ok());
        // Framing borrows a delivery position; it does not mutate any cursor.
        assert_eq!(cursor, 42);
        assert!(frame(cursor, cursor, "{}").is_err());
        assert!(frame(cursor, 41, "{}").is_err());
    }

    #[test]
    fn sse_injection_and_oversize_refuse() {
        assert!(frame(0, 1, "{}\nid: 999").is_err());
        assert!(frame(0, 1, "{}\rdata: forged").is_err());
        assert!(frame(0, 1, &"x".repeat(MAX_FRAME_BYTES)).is_err());
        assert_eq!(
            frame(0, 1, "{}"),
            Ok("id: 1\nevent: quipu.event\ndata: {}\n\n".into())
        );
    }
}
