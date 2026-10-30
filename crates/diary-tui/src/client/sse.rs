//! Hand-written, incremental SSE frame parser matching the server's exact
//! wire format (`id: {id}\nevent: {type}\ndata: {json}\n\n`, see
//! `diary-control::server::sse_bytes`). Parses only the event fields needed by the TUI and
//! enforces a bound on event size so no per-event window can grow forever.

use diary_control_contract::EventDto;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SseError {
    OversizedEvent,
    MalformedEvent,
}

/// Feeds raw bytes incrementally and yields complete, decoded events.
/// Holds only the bytes of the current in-progress event, never the whole
/// stream.
pub struct SseParser {
    buffer: Vec<u8>,
    max_event_bytes: usize,
}

impl SseParser {
    #[must_use]
    pub const fn new(max_event_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_event_bytes,
        }
    }

    /// Appends `chunk` and returns every complete event frame
    /// (`\n\n`-terminated) parsed out of the accumulated buffer, in order.
    /// On any error the parser's buffer is cleared, since the stream must
    /// be reconnected rather than resynchronized byte-by-byte.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<EventDto>, SseError> {
        self.buffer.extend_from_slice(chunk);

        let mut events = Vec::new();
        while let Some(boundary) = find_double_newline(&self.buffer) {
            if boundary > self.max_event_bytes {
                self.buffer.clear();
                return Err(SseError::OversizedEvent);
            }
            let frame = self.buffer[..boundary].to_vec();
            let rest_start = boundary + 2;
            self.buffer.drain(..rest_start);
            if frame.is_empty() {
                continue;
            }
            if let Some(event) = parse_frame(&frame)? {
                events.push(event);
            }
        }
        if self.buffer.len() > self.max_event_bytes {
            self.buffer.clear();
            return Err(SseError::OversizedEvent);
        }
        Ok(events)
    }
}

fn find_double_newline(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|window| window == b"\n\n")
}

fn parse_frame(frame: &[u8]) -> Result<Option<EventDto>, SseError> {
    let text = std::str::from_utf8(frame).map_err(|_| SseError::MalformedEvent)?;
    if text.lines().all(|line| line.starts_with(':')) {
        return Ok(None);
    }
    let mut wire_id = None;
    let mut wire_type = None;
    let mut data_line = None;
    for line in text.split('\n') {
        if let Some(id) = line.strip_prefix("id: ") {
            wire_id = id.parse::<u64>().ok();
        } else if let Some(event_type) = line.strip_prefix("event: ") {
            wire_type = Some(event_type);
        } else if let Some(data) = line.strip_prefix("data: ") {
            data_line = Some(data);
        }
    }
    let data = data_line.ok_or(SseError::MalformedEvent)?;
    let event: EventDto = serde_json::from_str(data).map_err(|_| SseError::MalformedEvent)?;
    if wire_id != Some(event.id) || wire_type != Some(event.event_type.as_str()) {
        return Err(SseError::MalformedEvent);
    }
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: u64, event_type: &str) -> Vec<u8> {
        let json = serde_json::json!({
            "id": id,
            "instance_id": "instance_synthetic",
            "event_type": event_type,
            "workflow_id": "wf_20261029",
            "revision": 3,
            "job_id": null,
            "progress_percent": null
        })
        .to_string();
        format!("id: {id}\nevent: {event_type}\ndata: {json}\n\n").into_bytes()
    }

    #[test]
    fn a_single_complete_frame_decodes_to_one_event() {
        let mut parser = SseParser::new(4096);
        let events = parser.feed(&frame(1, "workflow-changed")).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, 1);
        assert_eq!(events[0].event_type, "workflow-changed");
    }

    #[test]
    fn a_frame_split_across_two_chunks_still_decodes() {
        let full = frame(2, "job-progressed");
        let (first, second) = full.split_at(full.len() / 2);
        let mut parser = SseParser::new(4096);
        assert!(parser.feed(first).unwrap().is_empty());
        let events = parser.feed(second).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, 2);
    }

    #[test]
    fn two_frames_in_one_chunk_decode_in_order() {
        let mut combined = frame(3, "workflow-changed");
        combined.extend(frame(4, "job-progressed"));
        let mut parser = SseParser::new(4096);
        let events = parser.feed(&combined).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 3);
        assert_eq!(events[1].id, 4);
    }

    #[test]
    fn a_chunk_may_exceed_the_bound_when_each_complete_frame_does_not() {
        let first = frame(3, "workflow.changed");
        let second = frame(4, "job.changed");
        let frame_bound = first.len().max(second.len());
        let mut combined = first;
        combined.extend(second);
        assert!(combined.len() > frame_bound);
        let mut parser = SseParser::new(frame_bound);
        assert_eq!(parser.feed(&combined).unwrap().len(), 2);
    }

    #[test]
    fn an_oversized_event_is_rejected_and_clears_the_buffer() {
        let one_frame = frame(5, "workflow-changed");
        let mut parser = SseParser::new(one_frame.len() + 10);
        // An unterminated, over-budget partial frame (no `\n\n` yet) must
        // be rejected rather than buffered indefinitely.
        let padding = vec![b'x'; one_frame.len() + 20];
        let error = parser.feed(&padding).unwrap_err();
        assert_eq!(error, SseError::OversizedEvent);
        // The parser must have dropped the oversized partial frame rather
        // than resynchronizing within it: a normal frame afterward still
        // decodes.
        let events = parser.feed(&frame(6, "workflow-changed")).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, 6);
    }

    #[test]
    fn a_malformed_data_payload_is_rejected() {
        let mut parser = SseParser::new(4096);
        let bad = b"id: 7\nevent: workflow-changed\ndata: not json\n\n";
        assert_eq!(parser.feed(bad).unwrap_err(), SseError::MalformedEvent);
    }

    #[test]
    fn a_heartbeat_comment_is_ignored() {
        let mut parser = SseParser::new(4096);
        assert!(parser.feed(b": keep-alive\n\n").unwrap().is_empty());
    }

    #[test]
    fn wire_id_and_type_must_match_the_typed_payload() {
        let mut parser = SseParser::new(4096);
        let mut mismatched = frame(8, "workflow.changed");
        mismatched[4] = b'9';
        assert_eq!(
            parser.feed(&mismatched).unwrap_err(),
            SseError::MalformedEvent
        );
    }
}
