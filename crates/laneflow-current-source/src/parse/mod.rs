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

use anchor::{root_consumed_end, root_token_range};
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
/// 版本裁决。根不捕获为 token；根区间起点取首非空白 byte，终点在 walk 成功
/// 后经 JSON 感知配对扫描求得（根容器闭括号之后），不捕获或 replay 根文档。
///
/// map 路径的 `expecting` 固定为旧头部 DTO 的 expecting 文本（`struct
/// WireVersionHeader`），使非 object 根的 invalid type 消息与旧 gate 逐字节
/// 一致；seq-form 根按 `fields` 声明序逐位置解码，`struct_expecting` 用于
/// 缺位时的 derive invalid length 文本（`struct X with N elements`）。
fn drive_root<'de, F>(
    input: &'de [u8],
    expected_version: &'static str,
    struct_expecting: &'static str,
    fields: &'static [&'static str],
    mut handler: F,
) -> Result<GateReport, ParseFailure>
where
    F: FnMut(&mut Ctx<'de, NoLocations>, &str, &'de RawValue, ByteRange, usize, &mut RootGate),
{
    count_root_driver();
    let mut ctx = Ctx::new(input, NoLocations);
    // 探测区间只在 walk 失败路径作锚；walk 成功后以实际消费边界重求终点。
    let probe_range = root_token_range(input);
    let mut gate = RootGate {
        format_version: None,
        deferred: None,
    };
    let mut failure = None;
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let gate_dispatch = |ctx: &mut Ctx<'de, NoLocations>,
                         key: &str,
                         value: &'de RawValue,
                         range: ByteRange,
                         mark: usize| {
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
    };
    // token 形态分派：`[` 按声明序走位置序列（derive struct seq-form 平价）；
    // 其余形态走 map（非 object 由 map 路径报头部 expecting 的 invalid type）。
    let is_seq = input
        .get(probe_range.start as usize)
        .is_some_and(|byte| *byte == b'[');
    let result = if is_seq {
        walk::drive_seq(
            &mut ctx,
            &mut failure,
            struct_expecting,
            fields,
            probe_range,
            &mut deserializer,
            gate_dispatch,
        )
    } else {
        walk::drive_object(
            &mut ctx,
            &mut failure,
            "struct WireVersionHeader",
            &mut deserializer,
            gate_dispatch,
        )
    };

    // ① 头部 shape 立即失败（根 walk 内 handler 唯一的中止来源）。
    if let Some(candidate) = failure {
        return Err(ParseFailure::Shape(candidate));
    }
    // ② 真实 serde 错误：syntax/EOF 立即失败；Data（非 object 根）归一为
    //    以探测区间为锚的 shape。
    if let Err(error) = result {
        return Err(match error.classify() {
            Category::Data => ParseFailure::Shape(ShapeCandidate {
                path: "$".to_owned(),
                message: walk::strip_position_suffix(error.to_string()),
                anchor: probe_range,
            }),
            Category::Io | Category::Syntax | Category::Eof => ParseFailure::Syntax {
                path: ctx.canonical_path().to_owned(),
                source: error,
            },
        });
    }
    // walk 成功：精确根区间（终点=根容器配对闭括号之后，不吃进 trailing
    // content），随后打根 hook（NoLocations 为 no-op）。
    let root_range = ByteRange::new(
        probe_range.start,
        root_consumed_end(input, probe_range.start),
    );
    ctx.policy_root(root_range);
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

    /// 冻结顺序：缺失 formatVersion（③）先于 trailing content（④）；根 span
    /// 终点止于根 object 闭括号，不吃进 trailing content（R1 T3/T7）。
    #[test]
    fn missing_format_version_precedes_trailing_content() {
        let (path, message, anchor) =
            shape_parts(parse_traffic(b"{\"units\": {}} trailing").expect_err("缺失版本字段"));
        assert_eq!(path, "$");
        assert_eq!(message, "missing field `formatVersion`");
        assert_eq!(anchor, ByteRange::new(0, 13), "span 终点止于配对闭括号");
    }

    /// 空输入：serde 报 column 0 的 EOF，parse 层透出原始 serde 错误（span 的
    /// 一基 clamp 在 anchor::point_span，见 anchor 测试）。
    #[test]
    fn empty_input_is_syntax_eof() {
        let failure = parse_traffic(b"").expect_err("空输入");
        match failure {
            ParseFailure::Syntax { path, source } => {
                assert_eq!(path, "$");
                assert_eq!((source.line(), source.column()), (1, 0));
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// seq-form 平价（R1 T1/T4）：`[1,2]` 根按位置序列解码，位置 0=头部
    /// formatVersion 的字符串检查立即失败，与旧 derive 行为逐字节一致。
    #[test]
    fn seq_form_root_decodes_positionally_with_head_gate() {
        let (path, message, anchor) =
            shape_parts(parse_traffic(b"[1,2]").expect_err("seq-form 根头部 shape"));
        assert_eq!(path, "[0]");
        assert_eq!(message, "invalid type: integer `1`, expected a string");
        assert_eq!(anchor, ByteRange::new(1, 2));

        // 非容器标量根仍走 map 路径报头部 expecting（行为不变）。
        let (path, message, _anchor) = shape_parts(parse_traffic(b"1").expect_err("标量根"));
        assert_eq!(path, "$");
        assert_eq!(
            message,
            "invalid type: integer `1`, expected struct WireVersionHeader"
        );
    }

    /// seq-form 平价：全位置形态根包被接受（嵌套 units/signals 也用位置形
    /// 态，extensions 位置 17 为 opaque object）。
    #[test]
    fn fully_positional_root_package_is_accepted() {
        let input = br#"["0.10", ["m", "s"], {"edges": []}, [], [], [], [], [], [], [], [], [], [], [], [], [[], [], [], []], {"areas": [], "spaces": []}, {}]"#;
        let wire = parse_traffic(input).expect("全位置形态根包必须被接受");
        assert_eq!(wire.format_version(), "0.10");
        assert_eq!(wire.units().distance(), "m");
        assert_eq!(wire.units().time(), "s");
        assert!(wire.signals().maneuver_gates().is_empty());
    }

    /// seq-form 平价：缺位报 derive invalid length（`struct X with N
    /// elements`），path 停在缺位索引；嵌套 record 同理。
    #[test]
    fn seq_form_short_sequence_reports_invalid_length() {
        // 根：18 字段只给 1 个。
        let (path, message, _anchor) =
            shape_parts(parse_traffic(br#"["0.10"]"#).expect_err("根缺位"));
        assert_eq!(path, "[1]");
        assert_eq!(
            message,
            "invalid length 1, expected struct WirePackage with 18 elements"
        );

        // 嵌套：units 只给 1 个位置。
        let input = minimal_traffic().replacen(
            r#""units": {"distance": "m", "time": "s"}"#,
            r#""units": ["m"]"#,
            1,
        );
        let (path, message, _anchor) =
            shape_parts(parse_traffic(input.as_bytes()).expect_err("units 缺位"));
        assert_eq!(path, "units[1]");
        assert_eq!(
            message,
            "invalid length 1, expected struct WireUnits with 2 elements"
        );
    }

    /// 最小合法 traffic object（map 形态），嵌套 seq-form 测试以 replacen 派生。
    fn minimal_traffic() -> String {
        r#"{"formatVersion": "0.10", "units": {"distance": "m", "time": "s"}, "laneGraph": {"edges": []}, "junctions": [], "movements": [], "maneuverPaths": [], "routes": [], "vehicleProfiles": [], "participantClasses": [], "facilityBands": [], "roadSections": [], "laneGroups": [], "roadCorridors": [], "accessRules": [], "waitingZones": [], "signals": {"stopLines": [], "maneuverGates": [], "groups": [], "controllers": []}, "parking": {"areas": [], "spaces": []}}"#.to_owned()
    }

    /// seq-form 平价：嵌套 record 抽样——units、descriptor、signals。
    #[test]
    fn nested_seq_form_records_are_accepted() {
        // units 位置形态。
        let input = minimal_traffic().replacen(
            r#""units": {"distance": "m", "time": "s"}"#,
            r#""units": ["m", "s"]"#,
            1,
        );
        let wire = parse_traffic(input.as_bytes()).expect("units seq-form");
        assert_eq!(wire.units().distance(), "m");

        // signals 位置形态。
        let input = minimal_traffic().replacen(
            r#""signals": {"stopLines": [], "maneuverGates": [], "groups": [], "controllers": []}"#,
            r#""signals": [[], [], [], []]"#,
            1,
        );
        parse_traffic(input.as_bytes()).expect("signals seq-form");

        // manifest descriptor 位置形态。
        let manifest = br#"{"formatVersion": "0.1", "traffic": ["a", "m", "d", 0], "spatial": ["b", "m", "d", 0]}"#;
        let wire = parse_manifest(manifest).expect("descriptor seq-form");
        assert_eq!(wire.traffic().artifact_ref(), "a");
        assert_eq!(wire.spatial().size(), 0);
    }

    /// seq-form 平价：untagged variant——corridor 元素位置 0 字符串即第一
    /// variant（Section）；signal control 按位置 0/1 确定性分派。
    #[test]
    fn untagged_variants_accept_seq_form() {
        // corridor：`["e1"]` → Section（第一 variant 胜出）。
        let input = minimal_traffic().replacen(
            r#""roadCorridors": []"#,
            r#""roadCorridors": [{"id": "c", "referenceSectionId": "s", "elements": [["e1"]]}]"#,
            1,
        );
        let wire = parse_traffic(input.as_bytes()).expect("corridor 元素 seq-form");
        let element = &wire.road_corridors()[0].elements()[0];
        assert_eq!(
            element.as_section().expect("第一 variant").section_id(),
            "e1"
        );

        // signal control：`["group", "g1"]` → Group。
        let gate = |control: &str| {
            minimal_traffic().replacen(
                r#""signals": {"stopLines": [], "maneuverGates": [], "groups": [], "controllers": []}"#,
                &format!(
                    r#""signals": {{"stopLines": [], "maneuverGates": [{{"id": "g", "maneuverPathId": "m", "transitionIndex": 0, "stopLineId": "s", "signalControl": {control}}}], "groups": [], "controllers": []}}"#
                ),
                1,
            )
        };
        let wire = parse_traffic(gate(r#"["group", "g1"]"#).as_bytes()).expect("Group seq-form");
        let control = wire.signals().maneuver_gates()[0].signal_control();
        assert_eq!(control.as_group().expect("Group variant").group_id(), "g1");

        // `["none"]` → None。
        let wire = parse_traffic(gate(r#"["none"]"#).as_bytes()).expect("None seq-form");
        let control = wire.signals().maneuver_gates()[0].signal_control();
        assert!(control.as_group().is_none(), "None variant");

        // derive 平价：None 单字段 variant 忽略多余位置（`["none", "ignored"]`）。
        let wire =
            parse_traffic(gate(r#"["none", "ignored"]"#).as_bytes()).expect("多余位置静默忽略");
        let control = wire.signals().maneuver_gates()[0].signal_control();
        assert!(control.as_group().is_none(), "None variant 忽略位置 1");
    }

    /// 转义 key 平价（R1 T5）：含 `\uXXXX` 转义的合法 key 解码后与明文 key
    /// 走同一路径；duplicate 检测按解码后 key。
    #[test]
    fn escaped_keys_decode_like_plain_keys() {
        // 根头部闸口的转义 formatVersion。
        let manifest = r#"{"formatVersion": "0.1", "traffic": {"artifactRef": "a", "mediaType": "m", "digest": "d", "size": 0}, "spatial": {"artifactRef": "b", "mediaType": "m", "digest": "d", "size": 0}}"#;
        parse_manifest(manifest.as_bytes()).expect("明文基线");
        let escaped = manifest.replace(r#""formatVersion""#, r#""format\u0056ersion""#);
        parse_manifest(escaped.as_bytes()).expect("转义 formatVersion 必须被接受");

        // 嵌套 record 的转义 key（units.distance）。
        let input = minimal_traffic().replacen(r#""distance""#, r#""dist\u0061nce""#, 1);
        parse_traffic(input.as_bytes()).expect("转义 distance 必须被接受");

        // duplicate 按解码后 key 检测。
        let (path, message, _anchor) = shape_parts(
            parse_manifest(br#"{"formatVersion": "0.1", "format\u0056ersion": "0.1"}"#)
                .expect_err("转义重复 formatVersion"),
        );
        assert_eq!(path, "$");
        assert_eq!(message, "duplicate field `formatVersion`");
    }

    /// extensions duplicate 检查（R1 T2/T6）：第二次 occurrence 报 record 级
    /// duplicate field；单次仍接受且内容不透明。
    #[test]
    fn extensions_rejects_duplicate_occurrence() {
        let accepted = minimal_traffic().replacen(
            r#""parking": {"areas": [], "spaces": []}}"#,
            r#""parking": {"areas": [], "spaces": []}, "extensions": {"any": [1, 2]}}"#,
            1,
        );
        parse_traffic(accepted.as_bytes()).expect("单次 extensions 必须被接受");

        let duplicated = minimal_traffic().replacen(
            r#""parking": {"areas": [], "spaces": []}}"#,
            r#""parking": {"areas": [], "spaces": []}, "extensions": {}, "extensions": {}}"#,
            1,
        );
        let (path, message, _anchor) =
            shape_parts(parse_traffic(duplicated.as_bytes()).expect_err("重复 extensions"));
        assert_eq!(path, "$");
        assert_eq!(message, "duplicate field `extensions`");
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
}
