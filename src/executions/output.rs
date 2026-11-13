use serde::{Deserialize, Serialize};
const RETAIN: usize = 64 * 1024;
pub const READ_LIMIT: usize = 4096;
#[derive(Default, Clone, Serialize, Deserialize)]
pub(super) struct Output {
    text: String,
    base: u64,
    reader: u64,
    seen: u64,
}
#[derive(Debug, Serialize)]
pub struct OutputRead {
    pub text: String,
    pub start_cursor: u64,
    pub next_cursor: u64,
    pub oldest_cursor: u64,
    pub end_cursor: u64,
    pub truncated: bool,
    pub retention_loss: bool,
    pub partial_first_line: bool,
    pub returned_lines: usize,
    pub complete_result: bool,
}
fn boundary(text: &str, mut at: usize) -> usize {
    while at < text.len() && !text.is_char_boundary(at) {
        at += 1;
    }
    at
}
impl Output {
    pub(super) fn validate(&self) -> Result<(), String> {
        let end = self
            .base
            .checked_add(self.text.len() as u64)
            .ok_or("Invalid execution output offsets")?;
        if self.text.len() > RETAIN || self.reader > end || self.seen > end {
            return Err("Invalid retained execution output".into());
        }
        Ok(())
    }

    pub fn append(&mut self, text: &str) {
        if text.len() >= RETAIN {
            let skip = boundary(text, text.len() - RETAIN);
            self.base += self.text.len() as u64 + skip as u64;
            self.text.clear();
            self.text.push_str(&text[skip..]);
            return;
        }
        self.text.push_str(text);
        if self.text.len() > RETAIN {
            let drop = boundary(&self.text, self.text.len() - RETAIN);
            self.text.drain(..drop);
            self.base += drop as u64;
        }
    }
    pub fn end(&self) -> u64 {
        self.base + self.text.len() as u64
    }
    pub fn retention_loss(&self) -> bool {
        self.seen < self.base
    }
    pub fn unread_lines(&self) -> usize {
        let from = boundary(
            &self.text,
            self.seen
                .saturating_sub(self.base)
                .min(self.text.len() as u64) as usize,
        );
        self.text[from..].lines().count()
    }
    pub fn read(&mut self, tail: Option<usize>, cursor: Option<u64>) -> Result<OutputRead, String> {
        if tail.is_some_and(|n| !(1..=200).contains(&n)) || tail.is_some() && cursor.is_some() {
            return Err("tail_lines must be 1–200 and cannot be combined with cursor".into());
        }
        let tail = if tail.is_none() && cursor.is_none() {
            Some(20)
        } else {
            tail
        };
        let requested = cursor.unwrap_or(self.reader);
        if requested > self.end() {
            return Err("Output cursor is beyond the retained result".into());
        }
        let loss = requested < self.base;
        let mut start = requested.saturating_sub(self.base) as usize;
        let mut partial = self.base > 0 && start == 0;
        if let Some(lines) = tail {
            start = self.text.len();
            let bytes = self.text.as_bytes();
            let mut found = 0;
            let mut at = bytes.len();
            if at > 0 && bytes[at - 1] == b'\n' {
                at -= 1;
            }
            while at > 0 {
                at -= 1;
                if bytes[at] == b'\n' {
                    found += 1;
                    if found == lines {
                        start = at + 1;
                        break;
                    }
                }
            }
            if found < lines {
                start = 0;
            }
            partial = self.base > 0 && start == 0;
        } else if !self.text.is_char_boundary(start) {
            return Err("Output cursor must be a UTF-8 boundary".into());
        }
        let mut end = self.text.len();
        let mut truncated = false;
        if end - start > READ_LIMIT {
            truncated = true;
            if tail.is_some() {
                start = boundary(&self.text, end - READ_LIMIT);
                partial = true;
            } else {
                end = start + READ_LIMIT;
                while !self.text.is_char_boundary(end) {
                    end -= 1;
                }
            }
        }
        if tail.is_none() {
            if let Some((at, _)) = self.text[start..end].match_indices('\n').nth(199) {
                let line_end = start + at + 1;
                if line_end < end {
                    end = line_end;
                    truncated = true;
                }
            }
        }
        let text = self.text[start..end].to_string();
        let next = self.base + end as u64;
        self.seen = self.seen.max(next);
        if tail.is_none() {
            self.reader = next;
        }
        Ok(OutputRead {
            returned_lines: text.lines().count(),
            text,
            start_cursor: self.base + start as u64,
            next_cursor: next,
            oldest_cursor: self.base,
            end_cursor: self.end(),
            truncated,
            retention_loss: self.base > 0 && (tail.is_some() || loss),
            partial_first_line: partial,
            complete_result: self.base == 0 && start == 0 && end == self.text.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_pages_bound_line_count_independently_of_bytes() {
        let mut output = Output::default();
        output.append(&"x\n".repeat(1000));
        let page = output.read(None, Some(0)).unwrap();
        assert_eq!(page.returned_lines, 200);
        assert!(page.truncated);
        assert_eq!(page.next_cursor, 400);
    }
}
#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn corrupt_persisted_output_offsets_are_rejected_without_overflow() {
        let output: Output = serde_json::from_value(
            serde_json::json!({"text":"x","base":u64::MAX,"reader":0,"seen":0}),
        )
        .unwrap();
        assert!(output.validate().is_err());
        let output: Output = serde_json::from_value(
            serde_json::json!({"text":"x".repeat(65537),"base":0,"reader":0,"seen":0}),
        )
        .unwrap();
        assert!(output.validate().is_err());
    }
}
