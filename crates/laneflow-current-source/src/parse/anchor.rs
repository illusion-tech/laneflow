//! source 位置锚点：零基 byte 区间到一基 line/column span 的映射。
//!
//! span 语义（docs/design/current-package-import.md §10）：区间是**包含式**
//! 起止位置；行按 LF 递增（CR 归入前行）；column 按 byte 计（多字节 UTF-8
//! 序列逐 byte 递增）；非空 token 的 end 取 `end - 1` 的位置。

use crate::error::{CurrentSourcePosition, CurrentSourceSpan};

/// 原始输入中的零基半开 byte 区间 `[start, end)`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ByteRange {
    pub(crate) start: u32,
    pub(crate) end: u32,
}

impl ByteRange {
    pub(crate) const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

/// JSON 空白字节（space/tab/LF/CR，RFC 8259 §2）。
fn is_json_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn saturate(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

/// 根 token 区间：首/末非空白 byte。只在根 walk 成功后被调用（此时输入必然
/// 含非空白内容）；防御性退化不影响可达路径。
pub(crate) fn root_token_range(input: &[u8]) -> ByteRange {
    let start = input
        .iter()
        .position(|byte| !is_json_whitespace(*byte))
        .unwrap_or(0);
    let end = input
        .iter()
        .rposition(|byte| !is_json_whitespace(*byte))
        .map_or(start, |index| index + 1);
    ByteRange::new(saturate(start), saturate(end))
}

/// 单次 allocation-free 前缀扫描求 `[start, end-1]` 两个一基位置的 span。
///
/// 只对原始输入扫描一次：先记录 `start` 处位置，再记录 `end - 1` 处位置
/// （非空 token 的包含式终点；空区间防御性回退到 `start`）。
pub(crate) fn range_span(input: &[u8], range: ByteRange) -> CurrentSourceSpan {
    let start = range.start as usize;
    let end = (range.end as usize).saturating_sub(1).max(start);
    let mut line: u32 = 1;
    let mut column: u32 = 1;
    let mut start_position = (line, column);
    let mut index = 0_usize;
    while index <= end && index < input.len() {
        if index == start {
            start_position = (line, column);
        }
        if index == end {
            return make_span(start_position, (line, column));
        }
        if input[index] == b'\n' {
            line = line.saturating_add(1);
            column = 1;
        } else {
            // CR 与多字节 UTF-8 序列都按 byte 计入前行的 column。
            column = column.saturating_add(1);
        }
        index += 1;
    }
    // 区间越界只可能由内部不变量破坏引起；以扫描末尾防御性收尾。
    make_span(start_position, (line, column))
}

/// serde 一基位置的单点 span（JSON syntax 失败冻结为单点）。
pub(crate) fn point_span(line: usize, column: usize) -> CurrentSourceSpan {
    make_span(
        (saturate(line), saturate(column)),
        (saturate(line), saturate(column)),
    )
}

fn make_span(start: (u32, u32), end: (u32, u32)) -> CurrentSourceSpan {
    CurrentSourceSpan::new(
        CurrentSourcePosition::new(start.0, start.1),
        CurrentSourcePosition::new(end.0, end.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positions(span: CurrentSourceSpan) -> ((u32, u32), (u32, u32)) {
        (
            (span.start().line(), span.start().column()),
            (span.end().line(), span.end().column()),
        )
    }

    #[test]
    fn range_span_counts_columns_by_bytes_and_lines_by_lf() {
        // ASCII 单行：token 区间 [6,7) → 1:7 单点。
        assert_eq!(
            positions(range_span(br#"{"a": 1}"#, ByteRange::new(6, 7))),
            ((1, 7), (1, 7))
        );
        // 多字节 UTF-8 逐 byte 计列：`"值"` 占 [5,10)（值为 3 byte）。
        assert_eq!(
            positions(range_span(
                "{\"a\":\"值\"}".as_bytes(),
                ByteRange::new(5, 10)
            )),
            ((1, 6), (1, 10))
        );
        // 转义序列按原始 byte 计列：`"\n"` 字面量占 [5,9)。
        assert_eq!(
            positions(range_span(br#"{"a":"\n"}"#, ByteRange::new(5, 9))),
            ((1, 6), (1, 9))
        );
        // LF 增行：第二行第 8 列的 `1`。
        assert_eq!(
            positions(range_span(b"{\n  \"a\": 1\n}", ByteRange::new(9, 10))),
            ((2, 8), (2, 8))
        );
        // CRLF：CR 计入前行 column，LF 增行。
        assert_eq!(
            positions(range_span(b"{\r\n\"a\": 1}", ByteRange::new(8, 9))),
            ((2, 6), (2, 6))
        );
        // EOF：区间终点即输入最后一个 byte（无 trailing byte）。
        assert_eq!(
            positions(range_span(br#"{"a": 1}"#, ByteRange::new(0, 8))),
            ((1, 1), (1, 8))
        );
    }

    #[test]
    fn root_token_range_covers_empty_containers_and_trims_whitespace() {
        // 空 object / 空 array 根：包含式 span 覆盖两个 byte。
        let empty_object = root_token_range(b"{}");
        assert_eq!(empty_object, ByteRange::new(0, 2));
        assert_eq!(positions(range_span(b"{}", empty_object)), ((1, 1), (1, 2)));
        let empty_array = root_token_range(b"[]");
        assert_eq!(empty_array, ByteRange::new(0, 2));
        assert_eq!(positions(range_span(b"[]", empty_array)), ((1, 1), (1, 2)));
        // trailing/leading 空白不入区间（trailing 场景 span 止于 `}`）。
        let trimmed = root_token_range(b"  {} \n");
        assert_eq!(trimmed, ByteRange::new(2, 4));
        assert_eq!(positions(range_span(b"  {} \n", trimmed)), ((1, 3), (1, 4)));
    }

    #[test]
    fn point_span_is_a_single_point() {
        assert_eq!(positions(point_span(3, 14)), ((3, 14), (3, 14)));
    }
}
