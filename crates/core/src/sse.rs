//! Server-sent events: turns a `text/event-stream` body into events, as browsers' `EventSource` does
//! (https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation).

use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// `message` unless the server named it.
    pub event: String,
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A new reconnection time, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<u64>,
}

/// Feed it chunks as they arrive; it returns the events they complete.
#[derive(Debug, Default)]
pub struct Parser {
    /// Bytes of a line not finished yet (it may end in the middle of a UTF-8 character).
    pending: Vec<u8>,
    /// A `\r` ended the last chunk: a `\n` starting the next one belongs to it.
    after_cr: bool,
    started: bool,
    event: String,
    data: Vec<String>,
    id: Option<String>,
    retry: Option<u64>,
}

impl Parser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        let mut chunk = chunk;
        if self.after_cr && chunk.first() == Some(&b'\n') {
            chunk = &chunk[1..];
        }
        self.after_cr = false;
        let mut start = 0;
        let mut i = 0;
        while i < chunk.len() {
            match chunk[i] {
                b'\n' | b'\r' => {
                    self.pending.extend_from_slice(&chunk[start..i]);
                    let line = std::mem::take(&mut self.pending);
                    self.line(&String::from_utf8_lossy(&line), &mut out);
                    if chunk[i] == b'\r' {
                        if i + 1 < chunk.len() {
                            if chunk[i + 1] == b'\n' {
                                i += 1;
                            }
                        } else {
                            self.after_cr = true;
                        }
                    }
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        self.pending.extend_from_slice(&chunk[start..]);
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<Event>) {
        let line = if self.started {
            line
        } else {
            line.trim_start_matches('\u{feff}')
        };
        self.started = true;
        if line.is_empty() {
            // A blank line dispatches the event, if it has data.
            if !self.data.is_empty() {
                out.push(Event {
                    event: if self.event.is_empty() {
                        "message".into()
                    } else {
                        std::mem::take(&mut self.event)
                    },
                    data: self.data.join("\n"),
                    id: self.id.clone(),
                    retry: self.retry.take(),
                });
            }
            self.data.clear();
            self.event.clear();
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = value.to_string(),
            "data" => self.data.push(value.to_string()),
            "id" if !value.contains('\0') => self.id = Some(value.to_string()),
            "retry" => self.retry = value.parse().ok(),
            _ => {}
        }
    }
}

/// Every event in a whole stream.
pub fn parse_all(body: &[u8]) -> Vec<Event> {
    let mut parser = Parser::default();
    let mut events = parser.feed(body);
    events.extend(parser.feed(b"\n\n"));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_across_chunks_and_line_endings() {
        let mut p = Parser::default();
        let mut events = p.feed(b"\xef\xbb\xbf: comment\r\nevent: update\r\ndata: {\"a\":");
        assert!(events.is_empty());
        events.extend(p.feed(b"1}\r"));
        events
            .extend(p.feed(b"\ndata:line two\r\nid: 7\r\n\r\ndata: plain\n\nretry: 500\ndata\n\n"));
        assert_eq!(
            events,
            [
                Event {
                    event: "update".into(),
                    data: "{\"a\":1}\nline two".into(),
                    id: Some("7".into()),
                    retry: None
                },
                Event {
                    event: "message".into(),
                    data: "plain".into(),
                    id: Some("7".into()),
                    retry: None
                },
                Event {
                    event: "message".into(),
                    data: "".into(),
                    id: Some("7".into()),
                    retry: Some(500)
                },
            ]
        );
        // An event without a blank line at the end is complete at the end of the stream.
        assert_eq!(parse_all("data: é\n".as_bytes())[0].data, "é");
        // A character split across chunks survives.
        let mut p = Parser::default();
        let bytes = "data: é\n\n".as_bytes();
        let mut events = p.feed(&bytes[..7]);
        events.extend(p.feed(&bytes[7..]));
        assert_eq!(events[0].data, "é");
    }
}
