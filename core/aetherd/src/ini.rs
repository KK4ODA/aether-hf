//! An INI file's bytes, read and edited in place: a setting's value is rewritten and every
//! other byte is kept, so a program's file comes through unchanged but for what is set. Bytes,
//! not text: a file may hold a name or a place in any encoding. `VarAC`'s profiles are written
//! with it ([`crate::varac`]); the host-program finder reads Winlink's programs' files with it
//! ([`crate::hostapps`]).

/// An INI file's bytes, edited in place: a setting's value is rewritten and every other byte
/// is kept, so the operator's profile comes through unchanged but for what is set. Bytes, not
/// text: a profile may hold a name or a place in any encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ini {
    bytes: Vec<u8>,
}

/// The section a line heads, if it heads one: `[NAME]`, a byte-order mark allowed before it.
fn section_of(line: &[u8]) -> Option<&[u8]> {
    let line = line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(line);
    let line = line.trim_ascii();
    if line.len() >= 2 && line.first() == Some(&b'[') && line.last() == Some(&b']') {
        Some(line[1..line.len() - 1].trim_ascii())
    } else {
        None
    }
}

/// Whether a line heads a section, by name in any case.
fn heads(line: &[u8], section: &str) -> bool {
    let Some(name) = section_of(line) else {
        return false;
    };
    name.eq_ignore_ascii_case(section.as_bytes())
}

/// Where the `=` of a line that sets `key` is, by name in any case.
fn key_end(line: &[u8], key: &str) -> Option<usize> {
    let equals = line.iter().position(|b| *b == b'=')?;
    let name = line[..equals].trim_ascii();
    name.eq_ignore_ascii_case(key.as_bytes()).then_some(equals)
}

/// Where the line after one ending at `end` starts, and whether the file ended there without
/// a line end (so a line added there needs one first).
fn after(bytes: &[u8], end: usize) -> (usize, bool) {
    if end >= bytes.len() {
        (bytes.len(), true)
    } else if bytes[end] == b'\r' {
        (end + 2, false)
    } else {
        (end + 1, false)
    }
}

impl Ini {
    /// A file's bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// The bytes as they stand.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Each line's start and end, its line end left out.
    fn lines(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = 0;
        for (i, byte) in self.bytes.iter().enumerate() {
            if *byte == b'\n' {
                let end = if i > start && self.bytes[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                };
                out.push((start, end));
                start = i + 1;
            }
        }
        if start < self.bytes.len() {
            out.push((start, self.bytes.len()));
        }
        out
    }

    /// Whether the file ends its lines as Windows does: a line added ends the same way.
    fn crlf(&self) -> bool {
        self.bytes.windows(2).any(|pair| pair == b"\r\n")
    }

    /// Whether the file has a section, by name in any case.
    #[must_use]
    pub fn has_section(&self, section: &str) -> bool {
        for (start, end) in self.lines() {
            if heads(&self.bytes[start..end], section) {
                return true;
            }
        }
        false
    }

    /// The sections' names, in the order the file has them, a name that comes twice once.
    #[must_use]
    pub fn sections(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (start, end) in self.lines() {
            if let Some(name) = section_of(&self.bytes[start..end]) {
                let name = String::from_utf8_lossy(name).into_owned();
                if !out.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    out.push(name);
                }
            }
        }
        out
    }

    /// Where a setting's value is: from the byte after its `=` to its line's end.
    fn find(&self, section: &str, key: &str) -> Option<(usize, usize)> {
        let mut inside = false;
        for (start, end) in self.lines() {
            let line = &self.bytes[start..end];
            if section_of(line).is_some() {
                inside = heads(line, section);
            } else if inside && let Some(equals) = key_end(line, key) {
                return Some((start + equals + 1, end));
            }
        }
        None
    }

    /// A setting's value, its spaces trimmed; names in any case.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        let (from, to) = self.find(section, key)?;
        let value = self.bytes[from..to].trim_ascii();
        Some(String::from_utf8_lossy(value).into_owned())
    }

    /// Put `with` in place of the bytes from `from` to `to`.
    fn replace(&mut self, from: usize, to: usize, with: &[u8]) {
        let mut bytes = Vec::with_capacity(self.bytes.len() + with.len());
        bytes.extend_from_slice(&self.bytes[..from]);
        bytes.extend_from_slice(with);
        bytes.extend_from_slice(&self.bytes[to..]);
        self.bytes = bytes;
    }

    /// Where a line added to a section goes: after its last line that is not blank, and
    /// whether a line end must come first.
    fn section_end(&self, section: &str) -> Option<(usize, bool)> {
        let mut inside = false;
        let mut found = None;
        for (start, end) in self.lines() {
            let line = &self.bytes[start..end];
            if let Some(name) = section_of(line) {
                if inside {
                    break;
                }
                inside = name.eq_ignore_ascii_case(section.as_bytes());
                if inside {
                    found = Some(after(&self.bytes, end));
                }
            } else if inside && !line.trim_ascii().is_empty() {
                found = Some(after(&self.bytes, end));
            }
        }
        found
    }

    /// Set a setting, and say whether that changed the file. One not there yet goes at its
    /// section's end, and a section not there yet at the file's.
    pub fn set(&mut self, section: &str, key: &str, value: &str) -> bool {
        if let Some((from, to)) = self.find(section, key) {
            if self.bytes[from..to].trim_ascii() == value.as_bytes() {
                return false;
            }
            self.replace(from, to, value.as_bytes());
            return true;
        }
        let newline: &[u8] = if self.crlf() { b"\r\n" } else { b"\n" };
        let mut line = Vec::new();
        if let Some((at, end_first)) = self.section_end(section) {
            if end_first {
                line.extend_from_slice(newline);
            }
            line.extend_from_slice(key.as_bytes());
            line.push(b'=');
            line.extend_from_slice(value.as_bytes());
            line.extend_from_slice(newline);
            self.replace(at, at, &line);
        } else {
            if !self.bytes.is_empty() && !self.bytes.ends_with(b"\n") {
                line.extend_from_slice(newline);
            }
            line.push(b'[');
            line.extend_from_slice(section.as_bytes());
            line.push(b']');
            line.extend_from_slice(newline);
            line.extend_from_slice(key.as_bytes());
            line.push(b'=');
            line.extend_from_slice(value.as_bytes());
            line.extend_from_slice(newline);
            self.bytes.extend_from_slice(&line);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sections_are_listed_once_each_in_the_files_order() {
        let ini = Ini::new(
            b"\xEF\xBB\xBF[Main]\r\na=1\r\n[N0CALL]\r\n[main]\r\nb=2\r\n[ Vara ]\r\n".to_vec(),
        );
        assert_eq!(ini.sections(), ["Main", "N0CALL", "Vara"]);
        assert_eq!(ini.get("vara", "x"), None);
        assert_eq!(ini.get("MAIN", "A").as_deref(), Some("1"));
        assert!(
            Ini::new(Vec::new()).sections().is_empty(),
            "{:?}",
            Ini::new(Vec::new()).sections()
        );
    }
}
