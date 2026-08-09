//! 手写 DeserializeSeed/visitor 单遍解析层：三文档共享增长前 visitor 与
//! record-token replay（docs/design/current-package-import.md §7/§10/§12）。
//!
//! 失败顺序冻结（与旧双遍 `gate_format_version` + `deserialize_json` 逐用例
//! 对齐）：① 根 walk 的 syntax/EOF（含 token 捕获失败）或头部 shape 立即失
//! 败（文档序先到者优先）→ ② `formatVersion` 缺失 → ③ trailing content →
//! ④ unsupported version → ⑤ 首个延迟 shape 候选 → ⑥ 缺字段（声明序）。

mod anchor;
mod scenario;
mod traffic;
mod walk;

pub(crate) use anchor::{ByteRange, point_span, range_span};
pub(crate) use scenario::{parse_manifest, parse_spatial};
pub(crate) use traffic::parse_traffic;
pub(crate) use walk::{NoLocations, ShapeCandidate};

use serde_json::error::Category;
use serde_json::value::RawValue;

use anchor::root_token_range;
use walk::Ctx;

/// 单遍解析失败。
#[derive(Debug)]
pub(crate) enum ParseFailure {
    /// JSON token/UTF-8/EOF/trailing content 无效：携带真实 serde 错误（一基
    /// 位置由调用方造单点 span）。
    Syntax {
        path: String,
        source: serde_json::Error,
    },
    /// JSON shape 无效：延迟候选（锚点区间由调用方造 span）。
    Shape(ShapeCandidate),
    /// `formatVersion` 不是当前接受的版本（冻结：path `$`、span None）。
    UnsupportedVersion {
        expected: &'static str,
        actual: String,
    },
}

/// 版本闸口状态：恰好一个合法字符串 `formatVersion` occurrence 参与版本裁
/// 决；延迟候选只保留文档序首个。
pub(crate) struct RootGate {
    format_version: Option<String>,
    deferred: Option<ShapeCandidate>,
}

impl RootGate {
    /// 记录首个延迟 shape 候选并继续遍历（后续候选不改变首错选择）。
    pub(crate) fn defer(&mut self, candidate: ShapeCandidate) {
        if self.deferred.is_none() {
            self.deferred = Some(candidate);
        }
    }
}

/// 根驱动产物：闸口状态与根 token 区间。
pub(crate) struct GateReport {
    pub(crate) gate: RootGate,
    pub(crate) root_range: ByteRange,
}

#[inline]
fn count_root_driver() {
    #[cfg(debug_assertions)]
    crate::counters::record_root_driver();
}

/// 根级 missing field 候选（path `$`，锚=根 token 区间）。
pub(crate) fn missing_root_field(field: &'static str, root_range: ByteRange) -> ParseFailure {
    ParseFailure::Shape(ShapeCandidate {
        path: "$".to_owned(),
        message: walk::missing_field_message(field),
        anchor: root_range,
    })
}

/// 根文档单遍驱动：流式 walk + `formatVersion` 头部闸口 + trailing 检查 +
/// 版本裁决。根不捕获为 token；根 object 区间经首/末非空白 byte 扫描求得。
///
/// `expecting` 固定为旧头部 DTO 的 expecting 文本（`struct WireVersionHeader`），
/// 使非 object 根的 invalid type 消息与旧 gate 逐字节一致。
fn drive_root<'de, F>(
    input: &'de [u8],
    expected_version: &'static str,
    mut handler: F,
) -> Result<GateReport, ParseFailure>
where
    F: FnMut(&mut Ctx<'de, NoLocations>, &str, &'de RawValue, ByteRange, usize, &mut RootGate),
{
    count_root_driver();
    let mut ctx = Ctx::new(input, NoLocations);
    let root_range = root_token_range(input);
    ctx.policy_root(root_range);
    let mut gate = RootGate {
        format_version: None,
        deferred: None,
    };
    let mut failure = None;
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let result = walk::drive_object(
        &mut ctx,
        &mut failure,
        "struct WireVersionHeader",
        &mut deserializer,
        |ctx, key, value, range, mark| {
            if key == "formatVersion" {
                // 缺失、显式 null、非字符串或重复 occurrence 都以头部 shape
                // 立即失败；只有恰好一个合法字符串 occurrence 参与版本裁决。
                if gate.format_version.is_some() {
                    return Err(ctx.candidate_at(
                        mark,
                        walk::duplicate_field_message("formatVersion"),
                        range,
                    ));
                }
                match walk::decode_scalar::<String, NoLocations>(ctx, value, range) {
                    Ok(version) => gate.format_version = Some(version),
                    Err(candidate) => return Err(candidate),
                }
                return Ok(());
            }
            handler(ctx, key, value, range, mark, &mut gate);
            Ok(())
        },
    );

    // ① 头部 shape 立即失败（根 walk 内 handler 唯一的中止来源）。
    if let Some(candidate) = failure {
        return Err(ParseFailure::Shape(candidate));
    }
    // ② 真实 serde 错误：syntax/EOF 立即失败；Data（非 object 根）归一为
    //    以根区间为锚的 shape。
    if let Err(error) = result {
        return Err(match error.classify() {
            Category::Data => ParseFailure::Shape(ShapeCandidate {
                path: "$".to_owned(),
                message: walk::strip_position_suffix(error.to_string()),
                anchor: root_range,
            }),
            Category::Io | Category::Syntax | Category::Eof => ParseFailure::Syntax {
                path: ctx.canonical_path().to_owned(),
                source: error,
            },
        });
    }
    // ③ formatVersion 缺失（先于 trailing，复刻旧 gate 在 end() 前报 missing）。
    let Some(format_version) = gate.format_version.clone() else {
        return Err(missing_root_field("formatVersion", root_range));
    };
    // ④ trailing content（旧 gate 的 end()，先于版本裁决与其他 shape）。
    if let Err(source) = deserializer.end() {
        return Err(ParseFailure::Syntax {
            path: "$".to_owned(),
            source,
        });
    }
    // ⑤ 版本裁决（先于其他 shape）。
    if format_version != expected_version {
        return Err(ParseFailure::UnsupportedVersion {
            expected: expected_version,
            actual: format_version,
        });
    }
    Ok(GateReport { gate, root_range })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape_parts(failure: ParseFailure) -> (String, String, ByteRange) {
        match failure {
            ParseFailure::Shape(candidate) => (candidate.path, candidate.message, candidate.anchor),
            other => panic!("expected Shape, got {other:?}"),
        }
    }

    #[test]
    fn root_eof_is_syntax_with_real_position() {
        let failure = parse_traffic(b"{").expect_err("截断输入必须失败");
        match failure {
            ParseFailure::Syntax { path, source } => {
                assert_eq!(path, "$");
                assert_eq!((source.line(), source.column()), (1, 1));
                assert!(source.to_string().contains("EOF while parsing"));
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    #[test]
    fn header_shape_rejects_null_non_string_and_duplicate_occurrence() {
        // 显式 null：invalid type，锚=null token。
        let (path, message, anchor) = shape_parts(
            parse_traffic(br#"{"formatVersion": null}"#).expect_err("null formatVersion"),
        );
        assert_eq!(path, "formatVersion");
        assert_eq!(message, "invalid type: null, expected a string");
        assert_eq!(anchor, ByteRange::new(18, 22));

        // 非字符串：invalid type，锚=数字 token。
        let (path, message, anchor) =
            shape_parts(parse_traffic(br#"{"formatVersion": 1}"#).expect_err("数字 formatVersion"));
        assert_eq!(path, "formatVersion");
        assert_eq!(message, "invalid type: integer `1`, expected a string");
        assert_eq!(anchor, ByteRange::new(18, 19));

        // 重复 occurrence：path 归 record 级（`$`），锚=第二次 value token。
        let (path, message, anchor) = shape_parts(
            parse_traffic(br#"{"formatVersion": "0.10", "formatVersion": "0.10"}"#)
                .expect_err("重复 formatVersion"),
        );
        assert_eq!(path, "$");
        assert_eq!(message, "duplicate field `formatVersion`");
        assert_eq!(anchor, ByteRange::new(43, 49));
    }

    /// 冻结顺序：缺失 formatVersion（③）先于 trailing content（④）。
    #[test]
    fn missing_format_version_precedes_trailing_content() {
        let (path, message, _anchor) =
            shape_parts(parse_traffic(b"{\"units\": {}} trailing").expect_err("缺失版本字段"));
        assert_eq!(path, "$");
        assert_eq!(message, "missing field `formatVersion`");
    }

    /// 冻结顺序：trailing content（④）先于版本裁决（⑤）。
    #[test]
    fn trailing_content_precedes_version_decision() {
        let failure =
            parse_traffic(br#"{"formatVersion": "9.9"} x"#).expect_err("trailing content");
        match failure {
            ParseFailure::Syntax { path, source } => {
                assert_eq!(path, "$");
                assert!(source.to_string().starts_with("trailing characters"));
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// 冻结顺序：版本裁决（⑤）先于延迟 shape 候选（⑥）。
    #[test]
    fn version_decision_precedes_deferred_shape() {
        let failure =
            parse_traffic(br#"{"formatVersion": "9.9", "units": 1}"#).expect_err("不受支持的版本");
        match failure {
            ParseFailure::UnsupportedVersion { expected, actual } => {
                assert_eq!(expected, "0.10");
                assert_eq!(actual, "9.9");
            }
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    /// 冻结顺序：延迟 shape 候选（⑥）先于根缺字段（声明序）。
    #[test]
    fn deferred_shape_precedes_missing_root_fields() {
        let (path, message, anchor) = shape_parts(
            parse_manifest(br#"{"formatVersion": "0.1", "traffic": 1}"#)
                .expect_err("traffic 形状错误"),
        );
        assert_eq!(path, "traffic");
        assert_eq!(
            message,
            "invalid type: integer `1`, expected struct WireArtifactDescriptor"
        );
        assert_eq!(anchor, ByteRange::new(36, 37));
    }

    /// 根缺字段按 DTO 声明序报告（traffic 先于 spatial）。
    #[test]
    fn missing_root_fields_follow_declared_order() {
        let (path, message, _anchor) = shape_parts(
            parse_manifest(
                br#"{"formatVersion": "0.1", "spatial": {"artifactRef": "a", "mediaType": "m", "digest": "d", "size": 0}}"#,
            )
            .expect_err("缺失 traffic"),
        );
        assert_eq!(path, "$");
        assert_eq!(message, "missing field `traffic`");
    }

    /// 行为差异（报告记录）：derive 接受 struct 的 seq-form（`[1,2]`），新实现
    /// 只接受 object-form 根，统一以头部 DTO expecting 文本归一为 Shape。
    #[test]
    fn non_object_root_is_shape_with_header_expecting() {
        let (path, message, anchor) =
            shape_parts(parse_traffic(b"[1,2]").expect_err("seq-form 根"));
        assert_eq!(path, "$");
        assert_eq!(
            message,
            "invalid type: sequence, expected struct WireVersionHeader"
        );
        assert_eq!(anchor, ByteRange::new(0, 5));

        let (path, message, _anchor) = shape_parts(parse_traffic(b"1").expect_err("标量根"));
        assert_eq!(path, "$");
        assert_eq!(
            message,
            "invalid type: integer `1`, expected struct WireVersionHeader"
        );
    }
}
