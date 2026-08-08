//! Geometry v1 使用的有界 JSON 词法游标。
//!
//! 游标不构造通用 JSON value tree。后继 schema-specific parser 直接消费字符串、原始
//! 数字 token 与容器边界，并在相应 compact record 上冻结来源范围。

#![allow(
    dead_code,
    reason = "consumed by the following schema-specific parser slice"
)]

use std::sync::Arc;

use crate::SourceSpan;

const MAX_GEOMETRY_JSON_NESTING_DEPTH: u32 = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ByteSpan {
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum JsonErrorKind {
    Utf8Bom,
    InvalidUtf8,
    UnexpectedEnd,
    UnexpectedByte,
    InvalidStringEscape,
    InvalidUnicodeEscape,
    UnescapedControlCharacter,
    InvalidNumber,
    NestingDepthExceeded,
    TrailingBytes,
    SourcePositionOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct JsonError {
    pub(super) kind: JsonErrorKind,
    pub(super) span: ByteSpan,
}

pub(super) struct JsonCursor<'a> {
    source: &'a str,
    bytes: &'a [u8],
    offset: usize,
    depth: u32,
}

pub(super) struct LineIndex {
    line_starts: Box<[usize]>,
    source_len: usize,
}

impl LineIndex {
    pub(super) fn new(source: &[u8]) -> Result<Self, JsonError> {
        let mut line_starts = vec![0];
        let mut offset = 0_usize;
        while offset < source.len() {
            match source[offset] {
                b'\r' => {
                    offset += 1;
                    if source.get(offset) == Some(&b'\n') {
                        offset += 1;
                    }
                    line_starts.push(offset);
                }
                b'\n' => {
                    offset += 1;
                    line_starts.push(offset);
                }
                byte => offset += utf8_scalar_len(byte),
            }
        }
        if u32::try_from(line_starts.len()).is_err()
            || line_starts
                .last()
                .copied()
                .is_some_and(|start| u32::try_from(source.len().saturating_sub(start) + 1).is_err())
        {
            return Err(JsonError {
                kind: JsonErrorKind::SourcePositionOverflow,
                span: ByteSpan {
                    start: source.len(),
                    end: source.len(),
                },
            });
        }
        Ok(Self {
            line_starts: line_starts.into_boxed_slice(),
            source_len: source.len(),
        })
    }

    pub(super) fn source_span(&self, source_document_key: &Arc<str>, span: ByteSpan) -> SourceSpan {
        let start_offset = span.start.min(self.source_len);
        let end_offset = if span.end > span.start {
            span.end
                .saturating_sub(1)
                .min(self.source_len.saturating_sub(1))
        } else {
            start_offset
        };
        let (start_line, start_column) = self.position(start_offset);
        let (end_line, end_column) = self.position(end_offset);
        SourceSpan::range(
            Arc::clone(source_document_key),
            start_line,
            start_column,
            end_line,
            end_column,
        )
    }

    fn position(&self, offset: usize) -> (u32, u32) {
        let line_index = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let line_start = self.line_starts[line_index];
        (
            u32::try_from(line_index + 1).expect("line count checked at construction"),
            u32::try_from(offset.saturating_sub(line_start) + 1)
                .expect("line byte column checked at construction"),
        )
    }
}

impl<'a> JsonCursor<'a> {
    pub(super) fn new(source_bytes: &'a [u8]) -> Result<Self, JsonError> {
        if source_bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            return Err(JsonError {
                kind: JsonErrorKind::Utf8Bom,
                span: ByteSpan { start: 0, end: 3 },
            });
        }
        let source = std::str::from_utf8(source_bytes).map_err(|error| {
            let start = error.valid_up_to();
            let error_len = error.error_len().unwrap_or(1);
            JsonError {
                kind: JsonErrorKind::InvalidUtf8,
                span: ByteSpan {
                    start,
                    end: start.saturating_add(error_len).min(source_bytes.len()),
                },
            }
        })?;
        Ok(Self {
            source,
            bytes: source_bytes,
            offset: 0,
            depth: 0,
        })
    }

    pub(super) const fn offset(&self) -> usize {
        self.offset
    }

    pub(super) fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.offset += 1;
        }
    }

    pub(super) fn begin_object(&mut self) -> Result<ByteSpan, JsonError> {
        self.begin_container(b'{')
    }

    pub(super) fn end_object(&mut self) -> Result<ByteSpan, JsonError> {
        self.end_container(b'}')
    }

    pub(super) fn begin_array(&mut self) -> Result<ByteSpan, JsonError> {
        self.begin_container(b'[')
    }

    pub(super) fn end_array(&mut self) -> Result<ByteSpan, JsonError> {
        self.end_container(b']')
    }

    pub(super) fn consume_comma(&mut self) -> Result<ByteSpan, JsonError> {
        self.consume_byte(b',')
    }

    pub(super) fn consume_colon(&mut self) -> Result<ByteSpan, JsonError> {
        self.consume_byte(b':')
    }

    pub(super) fn next_is(&mut self, expected: u8) -> bool {
        self.skip_whitespace();
        self.peek() == Some(expected)
    }

    pub(super) fn parse_string(&mut self) -> Result<(String, ByteSpan), JsonError> {
        self.skip_whitespace();
        let start = self.offset;
        self.expect_raw_byte(b'"')?;
        let mut value = String::new();
        let mut chunk_start = self.offset;

        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error_at(JsonErrorKind::UnexpectedEnd, self.offset, self.offset));
            };
            match byte {
                b'"' => {
                    value.push_str(&self.source[chunk_start..self.offset]);
                    self.offset += 1;
                    return Ok((
                        value,
                        ByteSpan {
                            start,
                            end: self.offset,
                        },
                    ));
                }
                b'\\' => {
                    value.push_str(&self.source[chunk_start..self.offset]);
                    self.offset += 1;
                    self.parse_escape(&mut value)?;
                    chunk_start = self.offset;
                }
                0x00..=0x1f => {
                    return Err(self.error_at(
                        JsonErrorKind::UnescapedControlCharacter,
                        self.offset,
                        self.offset + 1,
                    ));
                }
                _ => self.offset += utf8_scalar_len(byte),
            }
        }
    }

    pub(super) fn parse_number_token(&mut self) -> Result<(&'a str, ByteSpan), JsonError> {
        self.skip_whitespace();
        let start = self.offset;
        if self.peek() == Some(b'-') {
            self.offset += 1;
        }

        match self.peek() {
            Some(b'0') => {
                self.offset += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.number_error(start));
                }
            }
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.offset += 1;
                }
            }
            _ => return Err(self.number_error(start)),
        }

        if self.peek() == Some(b'.') {
            self.offset += 1;
            let fraction_start = self.offset;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
            if self.offset == fraction_start {
                return Err(self.number_error(start));
            }
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            let exponent_start = self.offset;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
            if self.offset == exponent_start {
                return Err(self.number_error(start));
            }
        }

        let span = ByteSpan {
            start,
            end: self.offset,
        };
        Ok((&self.source[start..self.offset], span))
    }

    pub(super) fn parse_literal(&mut self, expected: &[u8]) -> Result<ByteSpan, JsonError> {
        self.skip_whitespace();
        let start = self.offset;
        let end = start.saturating_add(expected.len());
        if self.bytes.get(start..end) != Some(expected) {
            return Err(self.error_at(
                if end > self.bytes.len() {
                    JsonErrorKind::UnexpectedEnd
                } else {
                    JsonErrorKind::UnexpectedByte
                },
                start,
                end.min(self.bytes.len()),
            ));
        }
        self.offset = end;
        Ok(ByteSpan { start, end })
    }

    pub(super) fn finish(mut self) -> Result<(), JsonError> {
        self.skip_whitespace();
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(self.error_at(
                JsonErrorKind::TrailingBytes,
                self.offset,
                self.offset + utf8_scalar_len(self.bytes[self.offset]),
            ))
        }
    }

    fn begin_container(&mut self, expected: u8) -> Result<ByteSpan, JsonError> {
        self.skip_whitespace();
        let start = self.offset;
        let next_depth = self.depth.checked_add(1).ok_or_else(|| {
            self.error_at(
                JsonErrorKind::NestingDepthExceeded,
                start,
                start.saturating_add(1).min(self.bytes.len()),
            )
        })?;
        if next_depth > MAX_GEOMETRY_JSON_NESTING_DEPTH {
            return Err(self.error_at(
                JsonErrorKind::NestingDepthExceeded,
                start,
                start.saturating_add(1).min(self.bytes.len()),
            ));
        }
        let span = self.consume_byte(expected)?;
        self.depth = next_depth;
        Ok(span)
    }

    fn end_container(&mut self, expected: u8) -> Result<ByteSpan, JsonError> {
        let span = self.consume_byte(expected)?;
        debug_assert!(self.depth > 0, "schema parser balances JSON containers");
        self.depth -= 1;
        Ok(span)
    }

    fn consume_byte(&mut self, expected: u8) -> Result<ByteSpan, JsonError> {
        self.skip_whitespace();
        let start = self.offset;
        self.expect_raw_byte(expected)?;
        Ok(ByteSpan {
            start,
            end: self.offset,
        })
    }

    fn expect_raw_byte(&mut self, expected: u8) -> Result<(), JsonError> {
        match self.peek() {
            Some(actual) if actual == expected => {
                self.offset += 1;
                Ok(())
            }
            Some(actual) => Err(self.error_at(
                JsonErrorKind::UnexpectedByte,
                self.offset,
                self.offset + utf8_scalar_len(actual),
            )),
            None => Err(self.error_at(JsonErrorKind::UnexpectedEnd, self.offset, self.offset)),
        }
    }

    fn parse_escape(&mut self, output: &mut String) -> Result<(), JsonError> {
        let escape_start = self.offset.saturating_sub(1);
        let Some(byte) = self.peek() else {
            return Err(self.error_at(JsonErrorKind::UnexpectedEnd, self.offset, self.offset));
        };
        self.offset += 1;
        match byte {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{0008}'),
            b'f' => output.push('\u{000c}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => {
                let first = self.parse_hex_quad(escape_start)?;
                let scalar = if (0xd800..=0xdbff).contains(&first) {
                    if self.bytes.get(self.offset..self.offset.saturating_add(2)) != Some(b"\\u") {
                        return Err(self.error_at(
                            JsonErrorKind::InvalidUnicodeEscape,
                            escape_start,
                            self.offset,
                        ));
                    }
                    self.offset += 2;
                    let second = self.parse_hex_quad(escape_start)?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err(self.error_at(
                            JsonErrorKind::InvalidUnicodeEscape,
                            escape_start,
                            self.offset,
                        ));
                    }
                    0x1_0000 + (((first - 0xd800) as u32) << 10) + (second - 0xdc00) as u32
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return Err(self.error_at(
                        JsonErrorKind::InvalidUnicodeEscape,
                        escape_start,
                        self.offset,
                    ));
                } else {
                    first as u32
                };
                let Some(character) = char::from_u32(scalar) else {
                    return Err(self.error_at(
                        JsonErrorKind::InvalidUnicodeEscape,
                        escape_start,
                        self.offset,
                    ));
                };
                output.push(character);
            }
            _ => {
                return Err(self.error_at(
                    JsonErrorKind::InvalidStringEscape,
                    escape_start,
                    self.offset,
                ));
            }
        }
        Ok(())
    }

    fn parse_hex_quad(&mut self, escape_start: usize) -> Result<u16, JsonError> {
        let end = self.offset.saturating_add(4);
        let Some(bytes) = self.bytes.get(self.offset..end) else {
            return Err(self.error_at(
                JsonErrorKind::UnexpectedEnd,
                escape_start,
                self.bytes.len(),
            ));
        };
        let mut value = 0_u16;
        for byte in bytes {
            let nibble = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => {
                    return Err(self.error_at(
                        JsonErrorKind::InvalidUnicodeEscape,
                        escape_start,
                        end,
                    ));
                }
            };
            value = (value << 4) | u16::from(nibble);
        }
        self.offset = end;
        Ok(value)
    }

    const fn peek(&self) -> Option<u8> {
        if self.offset < self.bytes.len() {
            Some(self.bytes[self.offset])
        } else {
            None
        }
    }

    fn number_error(&self, start: usize) -> JsonError {
        self.error_at(
            JsonErrorKind::InvalidNumber,
            start,
            self.offset.saturating_add(1).min(self.bytes.len()),
        )
    }

    const fn error_at(&self, kind: JsonErrorKind, start: usize, end: usize) -> JsonError {
        JsonError {
            kind,
            span: ByteSpan { start, end },
        }
    }
}

const fn utf8_scalar_len(first_byte: u8) -> usize {
    match first_byte {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ByteSpan, JsonCursor, JsonErrorKind, LineIndex, MAX_GEOMETRY_JSON_NESTING_DEPTH};

    #[test]
    fn parses_exact_strings_numbers_literals_and_containers() {
        let mut cursor = JsonCursor::new(
            r#" { "text": "路\u7ebf\uD83D\uDE97\n", "number": -0.25e+2, "ok": true } "#.as_bytes(),
        )
        .unwrap();
        cursor.begin_object().unwrap();
        let (key, _) = cursor.parse_string().unwrap();
        assert_eq!(key, "text");
        cursor.consume_colon().unwrap();
        let (text, _) = cursor.parse_string().unwrap();
        assert_eq!(text, "路线🚗\n");
        cursor.consume_comma().unwrap();
        assert_eq!(cursor.parse_string().unwrap().0, "number");
        cursor.consume_colon().unwrap();
        assert_eq!(cursor.parse_number_token().unwrap().0, "-0.25e+2");
        cursor.consume_comma().unwrap();
        assert_eq!(cursor.parse_string().unwrap().0, "ok");
        cursor.consume_colon().unwrap();
        cursor.parse_literal(b"true").unwrap();
        cursor.end_object().unwrap();
        cursor.finish().unwrap();
    }

    #[test]
    fn rejects_bom_invalid_utf8_and_invalid_json_numbers() {
        assert_eq!(
            JsonCursor::new(&[0xef, 0xbb, 0xbf, b'{'])
                .err()
                .unwrap()
                .kind,
            JsonErrorKind::Utf8Bom
        );
        assert_eq!(
            JsonCursor::new(&[0xff]).err().unwrap().kind,
            JsonErrorKind::InvalidUtf8
        );

        for invalid in ["01", "-", "1.", "1e", "1e+"] {
            assert_eq!(
                JsonCursor::new(invalid.as_bytes())
                    .unwrap()
                    .parse_number_token()
                    .unwrap_err()
                    .kind,
                JsonErrorKind::InvalidNumber,
                "invalid token: {invalid}"
            );
        }
    }

    #[test]
    fn rejects_unpaired_surrogates_and_unknown_escapes() {
        for (source, expected) in [
            (r#""\uD800""#, JsonErrorKind::InvalidUnicodeEscape),
            (r#""\uDC00""#, JsonErrorKind::InvalidUnicodeEscape),
            (r#""\uD800\u0041""#, JsonErrorKind::InvalidUnicodeEscape),
            (r#""\x""#, JsonErrorKind::InvalidStringEscape),
        ] {
            assert_eq!(
                JsonCursor::new(source.as_bytes())
                    .unwrap()
                    .parse_string()
                    .unwrap_err()
                    .kind,
                expected
            );
        }
    }

    #[test]
    fn checks_depth_before_accepting_the_next_container() {
        let valid = format!(
            "{}{}",
            "[".repeat(MAX_GEOMETRY_JSON_NESTING_DEPTH as usize),
            "]".repeat(MAX_GEOMETRY_JSON_NESTING_DEPTH as usize)
        );
        let mut cursor = JsonCursor::new(valid.as_bytes()).unwrap();
        for _ in 0..MAX_GEOMETRY_JSON_NESTING_DEPTH {
            cursor.begin_array().unwrap();
        }
        for _ in 0..MAX_GEOMETRY_JSON_NESTING_DEPTH {
            cursor.end_array().unwrap();
        }
        cursor.finish().unwrap();

        let invalid = "[".repeat((MAX_GEOMETRY_JSON_NESTING_DEPTH + 1) as usize);
        let mut cursor = JsonCursor::new(invalid.as_bytes()).unwrap();
        for _ in 0..MAX_GEOMETRY_JSON_NESTING_DEPTH {
            cursor.begin_array().unwrap();
        }
        assert_eq!(
            cursor.begin_array().unwrap_err().kind,
            JsonErrorKind::NestingDepthExceeded
        );
    }

    #[test]
    fn line_index_uses_lf_crlf_cr_and_utf8_byte_columns() {
        let source = "a\r\n路\tb\rc\n".as_bytes();
        let index = LineIndex::new(source).unwrap();
        let key: Arc<str> = Arc::from("source/main");

        let multibyte = index.source_span(&key, ByteSpan { start: 3, end: 6 });
        assert_eq!(multibyte.start().line(), 2);
        assert_eq!(multibyte.start().column(), 1);
        assert_eq!(multibyte.end().line(), 2);
        assert_eq!(multibyte.end().column(), 3);

        let after_tab = index.source_span(&key, ByteSpan { start: 7, end: 8 });
        assert_eq!(after_tab.start().line(), 2);
        assert_eq!(after_tab.start().column(), 5);

        let after_cr = index.source_span(&key, ByteSpan { start: 9, end: 10 });
        assert_eq!(after_cr.start().line(), 3);
        assert_eq!(after_cr.start().column(), 1);
    }
}
