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

/// 根容器的实际消费边界（零基 end offset）：从 `start` 的 `{`/`[` 起做 JSON
/// 感知的配对扫描（字符串与转义整体跳过），返回配对闭括号之后的 offset。
/// 只在根 walk 成功后调用（此时根必为 object/array 且配对闭括号存在）。
pub(crate) fn root_consumed_end(input: &[u8], start: u32) -> u32 {
    let mut index = start as usize;
    let mut depth = 0_u32;
    while index < input.len() {
        match input[index] {
            b'"' => {
                index = skip_string(input, index);
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return saturate(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    // 不可达（walk 已成功）；防御性收尾。
    saturate(input.len())
}

/// 标量根 token 的实际终点（零基 end offset）：字符串经 `skip_string`；
/// `true`/`false`/`null` 定长；数字经 `json_number_end` 按 JSON number 语
/// 法求词素终点；其余形态防御性取 `start + 1`。只在根 walk 的 Data 失败路
/// 径（非 object 根的 invalid type）作锚，不吃进 trailing content（R3-8）。
pub(crate) fn root_scalar_end(input: &[u8], start: u32) -> u32 {
    let begin = start as usize;
    match input.get(begin) {
        Some(b'"') => saturate(skip_string(input, begin)),
        Some(b't') => saturate(begin + 4), // true
        Some(b'f') => saturate(begin + 5), // false
        Some(b'n') => saturate(begin + 4), // null
        Some(b'-' | b'0'..=b'9') => saturate(json_number_end(input, begin)),
        _ => saturate(begin + 1),
    }
}

/// 跳过从 `quote`（`"` byte）开始的 JSON 字符串，返回闭引号之后的 offset。
/// 反斜杠转义统一跳过两个 byte（`\\`、`\"`、`\uXXXX` 的 `u` 后均无裸引号）。
pub(crate) fn skip_string(input: &[u8], quote: usize) -> usize {
    let mut index = quote + 1;
    while index < input.len() {
        match input[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    index
}

/// 从 `begin`（`-` 或数字）起按 JSON number 语法求词素终点（零基 end
/// offset）：`-? (0 | [1-9][0-9]*) (\. [0-9]+)? ([eE] [+-]? [0-9]+)?`，在第
/// 一个不符合语法推进的 byte 处停（R4-2：字符类扫描会把 `1-2`/`1.2.3` 的
/// trailing 垃圾吃进锚）。起点已被 serde 成功解析为合法 number，语法推进
/// 必然完整，残缺分支（如 `1e`）只在 root walk Data 失败路径的防御兜底出
/// 现，停在语法允许的最远位置即可。
fn json_number_end(input: &[u8], begin: usize) -> usize {
    let mut index = begin;
    if input.get(index) == Some(&b'-') {
        index += 1;
    }
    // int：`0` 或 `[1-9][0-9]*`。
    match input.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => {
            index += 1;
            while input.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        _ => return index,
    }
    // frac：`\. [0-9]+`（`.` 后无数字则 `.` 不属于词素）。
    if input.get(index) == Some(&b'.') && input.get(index + 1).is_some_and(u8::is_ascii_digit) {
        index += 2;
        while input.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
    }
    // exp：`[eE] [+-]? [0-9]+`（指数标记后无数字则标记不属于词素）。
    if matches!(input.get(index), Some(b'e' | b'E')) {
        let mut cursor = index + 1;
        if matches!(input.get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
        }
        if input.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
            while input.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }
            index = cursor;
        }
    }
    index
}

/// serde 一基位置的单点 span（JSON syntax 失败冻结为单点）。serde 对空输入
/// 的 EOF 报 column 0；位置契约是一基，line/column 一律 clamp 到 ≥1。
pub(crate) fn point_span(line: usize, column: usize) -> CurrentSourceSpan {
    make_span(
        (saturate(line.max(1)), saturate(column.max(1))),
        (saturate(line.max(1)), saturate(column.max(1))),
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

    /// 空输入的 serde EOF 报 column 0；位置契约是一基，clamp 到 1:1。
    #[test]
    fn point_span_clamps_to_one_based() {
        assert_eq!(positions(point_span(1, 0)), ((1, 1), (1, 1)));
        assert_eq!(positions(point_span(0, 0)), ((1, 1), (1, 1)));
    }

    #[test]
    fn root_consumed_end_stops_at_matching_close() {
        // 字符串内的闭括号不影响配对；终点=根 `}` 之后（不含 trailing）。
        let input = br#"{"a": "}", "b": [1]} trailing"#;
        assert_eq!(root_consumed_end(input, 0), 20);
        // 嵌套容器与转义。
        let input = br#"  {"a": ["\"}"], "b": {}}  "#;
        assert_eq!(root_consumed_end(input, 2), input.len() as u32 - 2);
        // 根 array 形态。
        assert_eq!(root_consumed_end(br#"[1, 2] x"#, 0), 6);
    }
}
