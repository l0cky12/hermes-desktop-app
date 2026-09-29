//! The Client log: what this client did while talking to Hermes. Callers pass only method,
//! path, status, subcommand names, exit codes, and warnings, never content or secrets.

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::ipc::Channel;

const CAPACITY: usize = 2000;

#[derive(Serialize, Clone)]
pub struct Line {
    /// Unix epoch milliseconds.
    pub time: u64,
    pub source: &'static str,
    pub text: String,
}

pub struct Ring {
    capacity: usize,
    lines: VecDeque<Line>,
}

impl Ring {
    pub const fn new(capacity: usize) -> Self {
        Ring { capacity, lines: VecDeque::new() }
    }

    pub fn push(&mut self, line: Line) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter()
    }
}

static RING: Mutex<Ring> = Mutex::new(Ring::new(CAPACITY));
/// The Logs view, while it's listening.
static SINK: Mutex<Option<Channel<Line>>> = Mutex::new(None);

/// Records one Client log line and echoes it to stderr.
pub fn write(source: &'static str, text: impl Into<String>) {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let line = Line { time, source, text: text.into() };
    eprintln!("[{source}] {}", line.text);
    // The ring lock is held across the send so `subscribe` never misses or repeats a line.
    let mut ring = RING.lock().unwrap();
    if let Some(sink) = SINK.lock().unwrap().as_ref() {
        let _ = sink.send(line.clone());
    }
    ring.push(line);
}

/// Everything held so far; later lines go to `sink`.
pub fn subscribe(sink: Channel<Line>) -> Vec<Line> {
    let ring = RING.lock().unwrap();
    *SINK.lock().unwrap() = Some(sink);
    ring.lines().cloned().collect()
}

pub fn clear() {
    RING.lock().unwrap().lines.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_only_the_newest_lines_in_order() {
        let mut ring = Ring::new(3);
        for i in 0..5 {
            ring.push(Line { time: i, source: "gateway", text: format!("line {i}") });
        }
        let texts: Vec<_> = ring.lines().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["line 2", "line 3", "line 4"]);
    }
}
