//! The segments of a request path, percent-decoded: a flow step id such as
//! `ci@dev/ci_watch` or a plan task row such as `implement#t1` travels encoded.

const SEGMENT_SEPARATOR: char = '/';
const PERCENT: u8 = b'%';
const HEX_BASE: u8 = 16;
const ESCAPE_LEN: usize = 3;

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(u32::from(HEX_BASE))
        .and_then(|digit| u8::try_from(digit).ok())
}

/// The byte a `%XX` escape starting at `index` stands for, if it is one.
fn escaped_byte(bytes: &[u8], index: usize) -> Option<u8> {
    if bytes.get(index) != Some(&PERCENT) {
        return None;
    }
    let high = hex_digit(*bytes.get(index + 1)?)?;
    let low = hex_digit(*bytes.get(index + 2)?)?;
    Some(high * HEX_BASE + low)
}

/// One segment with its `%XX` escapes decoded; a malformed escape stays as written.
pub fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match escaped_byte(bytes, index) {
            Some(byte) => {
                out.push(byte);
                index += ESCAPE_LEN;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The decoded segments of `path` (already stripped of its prefix and slashes).
pub fn split(path: &str) -> Vec<String> {
    path.split(SEGMENT_SEPARATOR).map(decode).collect()
}

#[cfg(test)]
#[path = "../tests/unit/path_segments_tests.rs"]
mod tests;
