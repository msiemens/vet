/// Converts byte offsets of a text to UTF-16 units, counting `\r\n` and a lone `\r` as one unit.
/// Queries mostly ascend, so it walks forward from the last one.
pub(crate) struct Units<'a> {
    text: &'a str,
    byte: usize,
    unit: u32,
}

impl<'a> Units<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text, byte: 0, unit: 0 }
    }

    /// The offset in units of the character containing `byte`, or the text's length.
    pub fn at(&mut self, byte: usize) -> u32 {
        if byte < self.byte {
            (self.byte, self.unit) = (0, 0);
        }
        let bytes = self.text.as_bytes();
        for c in self.text[self.byte..].chars() {
            if self.byte >= byte {
                break;
            }
            // The `\r` of `\r\n` is part of the break the `\n` counts.
            if !(c == '\r' && bytes.get(self.byte + 1) == Some(&b'\n')) {
                self.unit += c.len_utf16() as u32;
            }
            self.byte += c.len_utf8();
        }
        self.unit
    }
}
