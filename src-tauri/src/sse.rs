//! The one SSE parser. Feed raw body chunks in, get complete events out.

#[derive(Debug, PartialEq)]
pub struct Event {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct Parser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl Parser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    out.push(Event { event: self.event.take(), data: self.data.join("\n") });
                    self.data.clear();
                }
                self.event = None;
            } else if line.starts_with(':') {
                // Comment line: the server's `: keepalive` heartbeat. Liveness only, never an event.
            } else {
                let (field, value) = line.split_once(':').unwrap_or((line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "event" => self.event = Some(value.to_owned()),
                    "data" => self.data.push(value.to_owned()),
                    _ => {} // `id:` and `retry:`; seq is read from the JSON payload instead.
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keepalive_comments_are_ignored() {
        let mut p = Parser::default();
        assert!(p.push(b": open\n\n: keepalive\n\n").is_empty());
        let ev = p.push(b"id: 1\ndata: {\"seq\":1}\n\n");
        assert_eq!(ev, vec![Event { event: None, data: "{\"seq\":1}".into() }]);
    }

    #[test]
    fn named_events_multiline_data_and_crlf() {
        let mut p = Parser::default();
        let ev = p.push(b"event: hermes.tool.progress\r\ndata: a\r\ndata: b\r\n\r\n");
        assert_eq!(ev, vec![Event { event: Some("hermes.tool.progress".into()), data: "a\nb".into() }]);
    }

    #[test]
    fn events_split_across_chunks_including_utf8() {
        let mut p = Parser::default();
        let bytes = "data: héllo\n\n".as_bytes();
        let (a, b) = bytes.split_at(8); // splits inside the two-byte 'é'
        assert!(p.push(a).is_empty());
        assert_eq!(p.push(b), vec![Event { event: None, data: "héllo".into() }]);
    }

    #[test]
    fn keepalive_between_fields_does_not_break_an_event() {
        let mut p = Parser::default();
        let ev = p.push(b"data: x\n: keepalive\ndata: y\n\n");
        assert_eq!(ev, vec![Event { event: None, data: "x\ny".into() }]);
    }
}
