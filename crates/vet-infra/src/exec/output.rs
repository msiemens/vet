//! Process output kept for error messages: the last lines of it.

use std::collections::VecDeque;

/// The last `capacity` non-blank lines pushed.
pub struct Tail {
    capacity: usize,
    lines: VecDeque<String>,
}

impl Tail {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            lines: VecDeque::new(),
        }
    }

    pub fn push(&mut self, line: String) {
        if line.trim().is_empty() {
            return;
        }
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(String::as_str).collect::<Vec<_>>().join("\n")
    }
}

/// Splits the chunks of a stream into the lines they complete.
#[derive(Default)]
pub struct LineSplitter {
    partial: Vec<u8>,
}

impl LineSplitter {
    /// The lines `chunk` completes, without their line ends; the rest waits for the next chunk.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.partial.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            lines.push(String::from_utf8_lossy(&line).trim_end().to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_keeps_the_last_lines_and_skips_blank_ones() {
        let mut tail = Tail::new(3);
        for line in ["one", "", "two", "   ", "three", "four"] {
            tail.push(line.to_owned());
        }
        assert_eq!(tail.text(), "two\nthree\nfour");
        let mut blank = Tail::new(3);
        blank.push(" ".to_owned());
        assert!(blank.is_empty());
    }

    #[test]
    fn lines_are_split_across_chunks() {
        let mut lines = LineSplitter::default();
        assert!(lines.feed(b"par").is_empty());
        assert_eq!(lines.feed(b"tial\r\nnext\nrest"), ["partial", "next"]);
        assert_eq!(lines.feed(b"\n"), ["rest"]);
    }
}
