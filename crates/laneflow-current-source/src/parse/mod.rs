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

use anchor::{root_consumed_end, root_scalar_end, root_token_range};
use walk::Ctx;

/// 单遍解析失败。
#[derive(Debug)]
pub(crate) enum ParseFailure {
    /// JSON token/UTF-8/EOF/trailing content 无效：携带真实 serde 错误（一基
    /// 位置由调用方造单点 span）。`position` 为重建的全局位置 override
    /// （extensions 内容 sink 或 replay 内 Syntax 类失败的 token 局部错
    /// 误）；`None` 时取 serde 错误自带位置。
    Syntax {
        path: String,
        source: serde_json::Error,
        position: Option<(u32, u32)>,
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
/// 决；延迟候选只保留文档序首个（shape 与 extensions 内容 syntax 分开记
/// 录，裁决后按文档位置取先到者——旧全量解析的首错语义）。
pub(crate) struct RootGate {
    format_version: Option<String>,
    deferred: Option<ShapeCandidate>,
    deferred_syntax: Option<walk::DeferredSyntax>,
}

impl RootGate {
    /// 是否已确立首个延迟失败（R4-3：确立后根驱动跳过后续字段物化，恢复旧
    /// 两遍 loader 的 fail-fast 拒绝成本；token 捕获的 syntax/trailing 校验
    /// 与 formatVersion 处理在 walk/闸口层不受影响）。
    pub(crate) fn has_deferred(&self) -> bool {
        self.deferred.is_some() || self.deferred_syntax.is_some()
    }

    /// 记录首个延迟 shape 候选并继续遍历（后续候选不改变首错选择）。
    pub(crate) fn defer(&mut self, candidate: ShapeCandidate) {
        if self.deferred.is_none() {
            self.deferred = Some(candidate);
        }
    }

    /// 记录首个延迟 syntax 失败（extensions 内容校验或 replay/捕获内
    /// Syntax 类失败，R3：保留原生 serde category）并继续遍历。
    pub(crate) fn defer_syntax(&mut self, failure: walk::DeferredSyntax) {
        if self.deferred_syntax.is_none() {
            self.deferred_syntax = Some(failure);
        }
    }

    /// 记录首个延迟 replay 失败：shape 与 syntax 分流到各自通道，
    /// `first_deferred` 统一按锚点文档序裁决先到者。
    pub(crate) fn defer_failure(&mut self, failure: walk::ReplayFailure) {
        match failure {
            walk::ReplayFailure::Shape(candidate) => self.defer(candidate),
            walk::ReplayFailure::Syntax(failure) => self.defer_syntax(failure),
        }
    }

    /// 版本裁决后的首个延迟失败：shape 与延迟 syntax 按文档位置取先到者
    /// （旧全量解析在文档序首个错误处失败的语义）。
    pub(crate) fn first_deferred(&mut self) -> Option<ParseFailure> {
        let deferred = self.deferred.take();
        let deferred_syntax = self.deferred_syntax.take();
        let syntax_start = deferred_syntax.as_ref().map(|failure| failure.token_start);
        let syntax = deferred_syntax.map(|failure| ParseFailure::Syntax {
            path: failure.path,
            source: failure.source,
            position: Some(failure.position),
        });
        match (deferred, syntax) {
            (None, None) => None,
            (Some(shape), None) => Some(ParseFailure::Shape(shape)),
            (None, Some(syntax)) => Some(syntax),
            (Some(shape), Some(syntax)) => {
                Some(if shape.anchor.start <= syntax_start.unwrap_or(u32::MAX) {
                    ParseFailure::Shape(shape)
                } else {
                    syntax
                })
            }
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

/// 根 seq-form 的头部闸口字段（旧 `WireVersionHeader` 仅 1 字段
/// formatVersion；R2 T1/T2：根 seq 多于 1 元素时 header `end_seq` 在第二元
/// 素处报 `trailing characters`，永远到不了全量解析）。
const HEADER_FIELDS: &[walk::FieldSpec] = &[walk::req("formatVersion")];

/// 根文档单遍驱动：流式 walk + `formatVersion` 头部闸口 + trailing 检查 +
/// 版本裁决。根不捕获为 token；根区间起点取首非空白 byte，终点在 walk 成功
/// 后经 JSON 感知配对扫描求得（根容器闭括号之后），不捕获或 replay 根文档。
///
/// map 路径的 `expecting` 固定为旧头部 DTO 的 expecting 文本（`struct
/// WireVersionHeader`），使非 object 根的 invalid type 消息与旧 gate 逐字节
/// 一致。seq-form 根复刻旧两阶段语义（R2 T1/T2 探针实证）：先以
/// `HEADER_FIELDS` 单字段走位置序列头部闸口（多于 1 元素由 serde_json
/// `end_seq` 报 trailing characters → JsonSyntax；空序列报 header invalid
/// length 0），版本裁决后 1 元素序列对 N 字段 struct 的结局是确定的（位置 1
/// 缺位），直接构造 invalid length 1 候选，不驱动第二次根 deserializer。
fn drive_root<'de, F>(
    input: &'de [u8],
    expected_version: &'static str,
    struct_expecting: &'static str,
    fields: &'static [walk::FieldSpec],
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
        deferred_syntax: None,
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
                return Err(ctx
                    .candidate_at(mark, walk::duplicate_field_message("formatVersion"), range)
                    .into());
            }
            match walk::decode_scalar::<String, NoLocations>(ctx, value, range) {
                Ok(version) => gate.format_version = Some(version),
                Err(failure) => return Err(failure),
            }
            return Ok(());
        }
        handler(ctx, key, value, range, mark, &mut gate);
        Ok(())
    };
    // token 形态分派：`[` 走 seq-form 头部闸口（HEADER_FIELDS 单字段位置序
    // 列；多于 1 元素由 `end_seq` 报 trailing characters）；其余形态走 map
    // （非 object 由 map 路径报头部 expecting 的 invalid type）。
    let is_seq = input
        .get(probe_range.start as usize)
        .is_some_and(|byte| *byte == b'[');
    let result = if is_seq {
        walk::drive_seq(
            &mut ctx,
            &mut failure,
            "struct WireVersionHeader",
            HEADER_FIELDS,
            probe_range,
            &mut deserializer,
            gate_dispatch,
        )
    } else {
        walk::drive_object(
            &mut ctx,
            &mut failure,
            "struct WireVersionHeader",
            probe_range.start,
            &mut deserializer,
            gate_dispatch,
        )
    };

    // ① 头部立即失败（根 walk 内 handler 唯一的中止来源）：shape 候选直接
    //    失败；头部 value 的 Syntax 类失败（如 `1e999` 的 number out of
    //    range）保留原生 category 立即失败（R3：全局位置 override 供 span）。
    if let Some(failure) = failure {
        return Err(match failure {
            walk::ReplayFailure::Shape(candidate) => ParseFailure::Shape(candidate),
            walk::ReplayFailure::Syntax(deferred) => ParseFailure::Syntax {
                path: deferred.path,
                source: deferred.source,
                position: Some(deferred.position),
            },
        });
    }
    // ② 真实 serde 错误：syntax/EOF 立即失败；Data（非 object 根）归一为
    //    以标量根 token 定界区间为锚的 shape（R3-8：字符串经 skip_string、
    //    true/false/null 定长、数字按 JSON 词法扫描，不吃进 trailing
    //    content）。
    if let Err(error) = result {
        return Err(match error.classify() {
            Category::Data => ParseFailure::Shape(ShapeCandidate {
                path: "$".to_owned(),
                message: walk::strip_position_suffix(error.to_string()),
                anchor: ByteRange::new(
                    probe_range.start,
                    root_scalar_end(input, probe_range.start),
                ),
            }),
            Category::Io | Category::Syntax | Category::Eof => ParseFailure::Syntax {
                path: ctx.canonical_path().to_owned(),
                source: error,
                position: None,
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
            position: None,
        });
    }
    // ⑤ 版本裁决（先于其他 shape）。
    if format_version != expected_version {
        return Err(ParseFailure::UnsupportedVersion {
            expected: expected_version,
            actual: format_version,
        });
    }
    // ⑥ seq-form 根：头部闸口通过即恰好 1 元素（多元素已在闸口以 trailing
    //    characters 失败）；1 元素对 N 字段 struct 的全量解析结局确定——位置
    //    1 缺位，直接构造 derive invalid length 1 候选（不驱动第二次根
    //    deserializer；探针实证 path `$`、category Data）。
    if is_seq {
        return Err(ParseFailure::Shape(ShapeCandidate {
            path: "$".to_owned(),
            message: walk::invalid_length_message(1, struct_expecting, fields.len()),
            anchor: root_range,
        }));
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
            ParseFailure::Syntax { path, source, .. } => {
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
            ParseFailure::Syntax { path, source, .. } => {
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

    /// 根 seq-form 两阶段平价（R2 T1/T2 探针实证）：多于 1 元素的根 seq 在
    /// header 闸口以 `trailing characters` 立即失败（JsonSyntax，真实 serde
    /// 位置）；根 seq 永远不成功。
    #[test]
    fn seq_root_with_extra_elements_is_trailing_characters_syntax() {
        let failure = parse_traffic(br#"["0.10",{}]"#).expect_err("两元素根 seq");
        match failure {
            ParseFailure::Syntax {
                path,
                source,
                position,
            } => {
                assert_eq!(path, "$");
                assert_eq!(position, None);
                assert_eq!((source.line(), source.column()), (1, 9));
                assert_eq!(source.to_string(), "trailing characters at line 1 column 9");
            }
            other => panic!("expected Syntax, got {other:?}"),
        }

        // 完整位置形态根包同样止步于 header 闸口（逗号后空白计入位置）。
        let input = br#"["0.10", ["m", "s"], {"edges": []}, [], [], [], [], [], [], [], [], [], [], [], [], [[], [], [], []], {"areas": [], "spaces": []}, {}]"#;
        let failure = parse_traffic(input).expect_err("完整位置根包必须被拒绝");
        match failure {
            ParseFailure::Syntax { path, source, .. } => {
                assert_eq!(path, "$");
                assert_eq!((source.line(), source.column()), (1, 10));
                assert_eq!(
                    source.to_string(),
                    "trailing characters at line 1 column 10"
                );
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// 根 seq 空序列（R2 T1/T2 探针实证）：header 位置 0 缺位 → derive
    /// invalid length 0（JsonShape，path `$`）。
    #[test]
    fn seq_root_empty_is_header_invalid_length_shape() {
        let (path, message, _anchor) = shape_parts(parse_traffic(b"[]").expect_err("空根 seq"));
        assert_eq!(path, "$");
        assert_eq!(
            message,
            "invalid length 0, expected struct WireVersionHeader with 1 element"
        );
    }

    /// 根 seq 版本裁决先于 invalid length（R2 T1/T2 探针实证）：`["9.9"]`
    /// header 成功 → UnsupportedFormatVersion。
    #[test]
    fn seq_root_bad_version_is_unsupported() {
        let failure = parse_traffic(br#"["9.9"]"#).expect_err("不受支持的版本");
        match failure {
            ParseFailure::UnsupportedVersion { expected, actual } => {
                assert_eq!(expected, "0.10");
                assert_eq!(actual, "9.9");
            }
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    /// 根 seq 恰好 1 元素（R2 T1/T2 探针实证）：header 成功、版本 OK → 全量
    /// struct 位置 1 缺位 → derive invalid length 1（JsonShape，path `$`）；
    /// 直接构造候选，不驱动第二次根 deserializer。
    #[test]
    fn seq_root_header_only_is_struct_invalid_length() {
        let (path, message, anchor) =
            shape_parts(parse_traffic(br#"["0.10"]"#).expect_err("traffic 根缺位"));
        assert_eq!(path, "$");
        assert_eq!(
            message,
            "invalid length 1, expected struct WirePackage with 18 elements"
        );
        assert_eq!(anchor, ByteRange::new(0, 8));

        let (path, message, _anchor) =
            shape_parts(parse_manifest(br#"["0.1"]"#).expect_err("manifest 根缺位"));
        assert_eq!(path, "$");
        assert_eq!(
            message,
            "invalid length 1, expected struct WireScenarioManifest with 3 elements"
        );

        let (path, message, _anchor) =
            shape_parts(parse_spatial(br#"["0.1"]"#).expect_err("spatial 根缺位"));
        assert_eq!(path, "$");
        assert_eq!(
            message,
            "invalid length 1, expected struct WireSpatialPackage with 3 elements"
        );
    }

    /// 嵌套 record 缺位报 derive invalid length（`struct X with N
    /// elements`），path 停在 record 级（serde_path_to_error 探针实证）。
    #[test]
    fn seq_form_short_sequence_reports_invalid_length() {
        // 嵌套：units 只给 1 个位置。
        let input = minimal_traffic().replacen(
            r#""units": {"distance": "m", "time": "s"}"#,
            r#""units": ["m"]"#,
            1,
        );
        let (path, message, _anchor) =
            shape_parts(parse_traffic(input.as_bytes()).expect_err("units 缺位"));
        assert_eq!(path, "units");
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
    }

    /// untagged 位置 variant 超元元素一律拒绝（R2 T4 探针实证）：超出所选
    /// variant 声明元数的元素使全部 variant 尝试失败，报 `data did not match
    /// any variant of untagged enum X`。
    #[test]
    fn untagged_seq_form_rejects_extra_elements() {
        // corridor：`["section", "unexpected"]` → 两个单字段 variant 都失败。
        let input = minimal_traffic().replacen(
            r#""roadCorridors": []"#,
            r#""roadCorridors": [{"id": "c", "referenceSectionId": "s", "elements": [["section", "unexpected"]]}]"#,
            1,
        );
        let (path, message, _anchor) =
            shape_parts(parse_traffic(input.as_bytes()).expect_err("corridor 超元元素"));
        assert_eq!(path, "roadCorridors[0].elements[0]");
        assert_eq!(
            message,
            "data did not match any variant of untagged enum WireCorridorElement"
        );

        let gate = |control: &str| {
            minimal_traffic().replacen(
                r#""signals": {"stopLines": [], "maneuverGates": [], "groups": [], "controllers": []}"#,
                &format!(
                    r#""signals": {{"stopLines": [], "maneuverGates": [{{"id": "g", "maneuverPathId": "m", "transitionIndex": 0, "stopLineId": "s", "signalControl": {control}}}], "groups": [], "controllers": []}}"#
                ),
                1,
            )
        };
        // `["none", "unexpected"]` → None 单字段 variant 因超元失败。
        let (path, message, _anchor) = shape_parts(
            parse_traffic(gate(r#"["none", "unexpected"]"#).as_bytes()).expect_err("none 超元元素"),
        );
        assert_eq!(path, "signals.maneuverGates[0].signalControl");
        assert_eq!(
            message,
            "data did not match any variant of untagged enum WireSignalControl"
        );

        // `["group", "g1", "extra"]` → Group 双字段 variant 因超元失败。
        let (_path, message, _anchor) = shape_parts(
            parse_traffic(gate(r#"["group", "g1", "extra"]"#).as_bytes())
                .expect_err("group 超元元素"),
        );
        assert_eq!(
            message,
            "data did not match any variant of untagged enum WireSignalControl"
        );
    }

    /// R4-1：untagged 元素内 Syntax 类 replay 失败（serde 数字词素溢出先于
    /// 类型检查）按 derive untagged 的 Content 缓冲语义传播，保留原生
    /// category 与字段级 path；只有 Data 类不匹配才归一为变体 mismatch。
    #[test]
    fn untagged_field_syntax_failure_propagates_with_native_category() {
        // corridor object-form：`{"sectionId":1e999}` → number out of range。
        let input = minimal_traffic().replacen(
            r#""roadCorridors": []"#,
            r#""roadCorridors": [{"id": "c", "referenceSectionId": "s", "elements": [{"sectionId":1e999}]}]"#,
            1,
        );
        match parse_traffic(input.as_bytes()).expect_err("sectionId 数字溢出") {
            ParseFailure::Syntax {
                path,
                source,
                position,
            } => {
                assert_eq!(path, "roadCorridors[0].elements[0].sectionId");
                assert!(
                    source.to_string().starts_with("number out of range"),
                    "消息：{source}"
                );
                assert!(position.is_some(), "全局 span override");
            }
            other => panic!("expected Syntax, got {other:?}"),
        }

        // signalControl object-form：`{"kind":"group","groupId":1e999}`。
        let input = minimal_traffic().replacen(
            r#""signals": {"stopLines": [], "maneuverGates": [], "groups": [], "controllers": []}"#,
            r#""signals": {"stopLines": [], "maneuverGates": [{"id": "g", "maneuverPathId": "m", "transitionIndex": 0, "stopLineId": "s", "signalControl": {"kind":"group","groupId":1e999}}], "groups": [], "controllers": []}"#,
            1,
        );
        match parse_traffic(input.as_bytes()).expect_err("groupId 数字溢出") {
            ParseFailure::Syntax { path, source, .. } => {
                assert_eq!(path, "signals.maneuverGates[0].signalControl.groupId");
                assert!(
                    source.to_string().starts_with("number out of range"),
                    "消息：{source}"
                );
            }
            other => panic!("expected Syntax, got {other:?}"),
        }

        // corridor seq-form：`[1e999]` → 位置 0 Syntax 传播。
        let input = minimal_traffic().replacen(
            r#""roadCorridors": []"#,
            r#""roadCorridors": [{"id": "c", "referenceSectionId": "s", "elements": [[1e999]]}]"#,
            1,
        );
        match parse_traffic(input.as_bytes()).expect_err("seq-form 数字溢出") {
            ParseFailure::Syntax { path, .. } => {
                assert_eq!(path, "roadCorridors[0].elements[0][0]");
            }
            other => panic!("expected Syntax, got {other:?}"),
        }

        // 回归守卫：Data 类不匹配（合法数字给 String 字段）仍归一 mismatch。
        let input = minimal_traffic().replacen(
            r#""roadCorridors": []"#,
            r#""roadCorridors": [{"id": "c", "referenceSectionId": "s", "elements": [{"sectionId":123}]}]"#,
            1,
        );
        let (path, message, _anchor) =
            shape_parts(parse_traffic(input.as_bytes()).expect_err("Data 类不匹配"));
        assert_eq!(path, "roadCorridors[0].elements[0]");
        assert_eq!(
            message,
            "data did not match any variant of untagged enum WireCorridorElement"
        );
    }

    /// serde(default) 位置语义（R2 T3 探针实证）：位置序列元素耗尽后，带
    /// `#[serde(default)]` 的剩余字段取默认值而非 invalid length；第一个无
    /// default 的剩余字段才报 invalid length（索引为字段声明位置）。
    #[test]
    fn positional_defaults_follow_serde_default_semantics() {
        // participant class：["pc"] —— extendsId（default）取 None。
        let input = minimal_traffic().replacen(
            r#""participantClasses": []"#,
            r#""participantClasses": [["pc"]]"#,
            1,
        );
        let wire = parse_traffic(input.as_bytes()).expect("participantClass 尾缺省");
        assert_eq!(wire.participant_classes()[0].id(), "pc");

        // access rule：只给前四必填字段 —— timeWindows/regulation/priority
        // （均 default）取 None。
        let input = minimal_traffic().replacen(
            r#""accessRules": []"#,
            r#""accessRules": [["r1", {"kind": "laneEdge", "id": "e1"}, "allow", ["pc"]]]"#,
            1,
        );
        let wire = parse_traffic(input.as_bytes()).expect("accessRule 尾缺省");
        assert_eq!(wire.access_rules()[0].id(), "r1");

        // regulation：["jurisdiction", "version"] —— source（default）取 None。
        let input = minimal_traffic().replacen(
            r#""accessRules": []"#,
            r#""accessRules": [{"id": "r1", "target": {"kind": "laneEdge", "id": "e1"}, "effect": "allow", "participantClassIds": ["pc"], "regulation": ["j", "v"]}]"#,
            1,
        );
        let wire = parse_traffic(input.as_bytes()).expect("regulation 尾缺省");
        assert_eq!(
            wire.access_rules()[0]
                .regulation()
                .expect("regulation 存在")
                .jurisdiction(),
            "j"
        );

        // parking space：areaId（位置 1，default）跳过，entry（位置 2，必填）
        // 缺位 → invalid length 2（索引为字段声明位置，非耗尽位置）。
        let input = minimal_traffic().replacen(
            r#""parking": {"areas": [], "spaces": []}"#,
            r#""parking": {"areas": [], "spaces": [["s1"]]}"#,
            1,
        );
        let (path, message, _anchor) =
            shape_parts(parse_traffic(input.as_bytes()).expect_err("space 必填缺位"));
        assert_eq!(path, "parking.spaces[0]");
        assert_eq!(
            message,
            "invalid length 2, expected struct WireParkingSpace with 5 elements"
        );

        // 反例：无 default 的尾字段（units.time）缺位仍报 invalid length。
        let input = minimal_traffic().replacen(
            r#""units": {"distance": "m", "time": "s"}"#,
            r#""units": ["m"]"#,
            1,
        );
        parse_traffic(input.as_bytes()).expect_err("无 default 尾字段仍 invalid length");
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

    /// 以 extensions 结尾的最小合法 traffic（`{"x": ...}` 内容）。
    fn traffic_with_extensions(value: &str) -> String {
        minimal_traffic().replacen(
            r#""parking": {"areas": [], "spaces": []}}"#,
            &format!(r#""parking": {{"areas": [], "spaces": []}}, "extensions": {value}}}"#),
            1,
        )
    }

    fn nested_arrays(depth: usize) -> String {
        let mut nested = String::new();
        for _ in 0..depth {
            nested.push('[');
        }
        nested.push('1');
        for _ in 0..depth {
            nested.push(']');
        }
        nested
    }

    /// extensions 内容数值 range（R2 T5 探针实证）：超 f64 数字以 JsonSyntax
    /// 立即于版本裁决后失败；path 深入 extensions 子树，span 为全局位置。
    #[test]
    fn extensions_content_validates_number_range() {
        let doc = traffic_with_extensions(r#"{"x": 1e999}"#);
        let lexeme_start = doc.find("1e999").expect("lexeme 在文档中") as u32;
        let failure = parse_traffic(doc.as_bytes()).expect_err("超 f64 数字必须被拒绝");
        match failure {
            ParseFailure::Syntax {
                path,
                source,
                position,
            } => {
                assert_eq!(path, "extensions.x");
                assert_eq!(source.classify(), Category::Syntax);
                // payload 为 token 局部错误（零拷贝直达驱动，R3-5；位置事实源
                // 恒为 span）；span 位置重建为全局（lexeme 末位之后）。
                assert_eq!(
                    source.to_string(),
                    "number out of range at line 1 column 11"
                );
                assert_eq!(position, Some((1, lexeme_start + 5)));
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// extensions 内容递归深度（R2 T5 探针实证）：serde_json 128 层递归预算
    /// 经 wrapper seq 抵消根 object 层，生效边界与旧全量解析逐层一致
    /// （125 接受 / 126 起拒绝；审阅用例 128/129 均拒绝）。
    #[test]
    fn extensions_content_enforces_recursion_limit() {
        for depth in [100_usize, 125] {
            let doc = traffic_with_extensions(&format!("{{\"x\": {}}}", nested_arrays(depth)));
            parse_traffic(doc.as_bytes()).unwrap_or_else(|failure| {
                panic!("深度 {depth} 必须被接受：{failure:?}");
            });
        }
        for depth in [126_usize, 128, 129] {
            let doc = traffic_with_extensions(&format!("{{\"x\": {}}}", nested_arrays(depth)));
            let failure = match parse_traffic(doc.as_bytes()) {
                Err(failure) => failure,
                Ok(_) => panic!("深度 {depth} 必须被拒绝"),
            };
            match failure {
                ParseFailure::Syntax {
                    path,
                    source,
                    position,
                } => {
                    assert!(path.starts_with("extensions.x[0]"), "深度 {depth} path");
                    assert_eq!(source.classify(), Category::Syntax);
                    assert!(
                        source.to_string().starts_with("recursion limit exceeded"),
                        "深度 {depth} 消息"
                    );
                    assert!(position.is_some(), "深度 {depth} 全局 span");
                }
                other => panic!("expected Syntax, got {other:?}"),
            }
        }
    }

    /// extensions 非 object 外壳（R1 语义不变）：invalid type shape，锚=value
    /// token。
    #[test]
    fn extensions_non_object_is_shape_invalid_type() {
        let doc = traffic_with_extensions("[1]");
        let (path, message, _anchor) =
            shape_parts(parse_traffic(doc.as_bytes()).expect_err("非 object extensions"));
        assert_eq!(path, "extensions");
        assert_eq!(message, "invalid type: sequence, expected a map");
    }

    /// 冻结顺序（R2 T5）：extensions 内容 syntax 失败延迟到版本裁决之后；
    /// 与延迟 shape 候选按文档位置取先到者。
    #[test]
    fn extensions_content_error_defers_and_orders_by_document_position() {
        // 版本裁决优先于 extensions 内容错误。
        let doc = traffic_with_extensions(r#"{"x": 1e999}"#).replacen(r#""0.10""#, r#""9.9""#, 1);
        match parse_traffic(doc.as_bytes()).expect_err("不受支持的版本") {
            ParseFailure::UnsupportedVersion { actual, .. } => assert_eq!(actual, "9.9"),
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }

        // units shape（文档序在前）先于 extensions 内容错误。
        let doc = traffic_with_extensions(r#"{"x": 1e999}"#).replacen(
            r#""units": {"distance": "m", "time": "s"}"#,
            r#""units": 1"#,
            1,
        );
        let (path, message, _anchor) =
            shape_parts(parse_traffic(doc.as_bytes()).expect_err("文档序先到者优先"));
        assert_eq!(path, "units");
        assert_eq!(
            message,
            "invalid type: integer `1`, expected struct WireUnits"
        );

        // extensions 在文档序靠前时其内容错误优先。
        let doc = minimal_traffic()
            .replacen(
                r#""formatVersion": "0.10", "#,
                r#""extensions": {"x": 1e999}, "formatVersion": "0.10", "#,
                1,
            )
            .replacen(
                r#""units": {"distance": "m", "time": "s"}"#,
                r#""units": 1"#,
                1,
            );
        let failure = parse_traffic(doc.as_bytes()).expect_err("extensions 先到者优先");
        match failure {
            ParseFailure::Syntax { path, .. } => assert_eq!(path, "extensions.x"),
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// 冻结顺序：trailing content（④）先于版本裁决（⑤）。
    #[test]
    fn trailing_content_precedes_version_decision() {
        let failure =
            parse_traffic(br#"{"formatVersion": "9.9"} x"#).expect_err("trailing content");
        match failure {
            ParseFailure::Syntax { path, source, .. } => {
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

    /// R3-1：typed 数字字段的 `1e999` 保留原生 Syntax category（`number out
    /// of range` 延迟 syntax），不再归一为 Shape。
    #[test]
    fn typed_numeric_field_out_of_range_is_deferred_syntax() {
        let input = minimal_traffic().replacen(
            r#""laneGraph": {"edges": []}"#,
            r#""laneGraph": {"edges": [{"id": "e", "length": 1e999, "speedLimit": 1, "connections": []}]}"#,
            1,
        );
        let lexeme_start = input.find("1e999").expect("lexeme 在文档中") as u32;
        let failure = parse_traffic(input.as_bytes()).expect_err("超 f64 typed 字段");
        match failure {
            ParseFailure::Syntax {
                path,
                source,
                position,
            } => {
                assert_eq!(path, "laneGraph.edges[0].length");
                assert_eq!(source.classify(), Category::Syntax);
                assert!(source.to_string().starts_with("number out of range"));
                // token 局部位置重建为全局（lexeme 末位）。
                assert_eq!(position, Some((1, lexeme_start + 5)));
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// R3-4：字符串枚举字段的非标量 token 保留原生 Syntax category（真实
    /// serde `expected value`），不再归一为 Shape。
    #[test]
    fn enum_field_non_string_scalar_is_deferred_syntax() {
        let input = minimal_traffic().replacen(
            r#""signals": {"stopLines": [], "maneuverGates": [], "groups": [], "controllers": []}"#,
            r#""signals": {"stopLines": [{"id": "s", "edgeId": "e", "location": 1}], "maneuverGates": [], "groups": [], "controllers": []}"#,
            1,
        );
        let failure = parse_traffic(input.as_bytes()).expect_err("非标量枚举");
        match failure {
            ParseFailure::Syntax {
                path,
                source,
                position,
            } => {
                assert_eq!(path, "signals.stopLines[0].location");
                assert_eq!(source.classify(), Category::Syntax);
                assert_eq!(source.to_string(), "expected value at line 1 column 1");
                assert!(position.is_some(), "全局 span override");
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// R3-6：位置 record 超元（`"units": ["m","s",0]`）保留原生 Syntax
    /// category（真实 serde `trailing characters`），不再归一为 Shape。
    #[test]
    fn positional_record_surplus_element_is_deferred_syntax() {
        let input = minimal_traffic().replacen(
            r#""units": {"distance": "m", "time": "s"}"#,
            r#""units": ["m", "s", 0]"#,
            1,
        );
        let failure = parse_traffic(input.as_bytes()).expect_err("位置 record 超元");
        match failure {
            ParseFailure::Syntax {
                path,
                source,
                position,
            } => {
                assert_eq!(path, "units");
                assert_eq!(source.classify(), Category::Syntax);
                assert!(
                    source.to_string().starts_with("trailing characters"),
                    "消息：{source}"
                );
                assert!(position.is_some(), "全局 span override");
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// R3-7：四轴 point 恢复 `[f64; 3]` derive 行为（第 4 元素报真实
    /// `trailing characters` Syntax）；两轴仍 `invalid length 2` Shape。
    #[test]
    fn point_axis_count_follows_fixed_array_derive_categories() {
        let spatial = |points: &str| {
            format!(
                r#"{{"formatVersion": "0.1", "frameId": "f", "edges": [{{"trafficEdgeId": "e", "centerline": {{"points": [{points}]}}}}]}}"#
            )
        };
        let failure = parse_spatial(spatial("[1,2,3,4]").as_bytes()).expect_err("四轴 point");
        match failure {
            ParseFailure::Syntax {
                path,
                source,
                position,
            } => {
                assert_eq!(path, "edges[0].centerline.points[0]");
                assert_eq!(source.classify(), Category::Syntax);
                assert!(
                    source.to_string().starts_with("trailing characters"),
                    "消息：{source}"
                );
                assert!(position.is_some(), "全局 span override");
            }
            other => panic!("expected Syntax, got {other:?}"),
        }

        let (path, message, _anchor) =
            shape_parts(parse_spatial(spatial("[1,2]").as_bytes()).expect_err("两轴 point"));
        assert_eq!(path, "edges[0].centerline.points[0]");
        assert_eq!(message, "invalid length 2, expected an array of length 3");
    }

    /// R3-3 dispute pin：字段 value 捕获期截断的 syntax 失败 path 保持
    /// field 级（`units`）——serde_path_to_error 在进入 value 解析前即追加
    /// key 段落，截断输入 `{"formatVersion":"0.10","units":` 旧 loader 报
    /// `units`（探针实证，冻结行为本就如此，不做 truncate 特判）。
    #[test]
    fn truncated_field_value_syntax_keeps_field_path() {
        let failure =
            parse_traffic(b"{\"formatVersion\":\"0.10\",\"units\":").expect_err("截断 value");
        match failure {
            ParseFailure::Syntax { path, position, .. } => {
                assert_eq!(path, "units");
                assert_eq!(position, None);
            }
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// R5：pre-value（冒号/分隔符阶段）syntax 失败归 record 级 path（探针
    /// 实证：serde_path_to_error 的 next_value_seed 在 delegate 失败时以
    /// parent chain 触发，key 段落只随 TrackedSeed 进入 value 解析）；冒号
    /// 已消费的 value 阶段失败保持字段级（与 R3-3 dispute pin 互补）。
    #[test]
    fn pre_value_syntax_failures_restore_record_path() {
        // 冒号未消费：key 后 EOF / 垃圾 / 闭括号 / 空白后 EOF / 首字段垃圾。
        for input in [
            &br#"{"formatVersion":"0.10","units""#[..],
            &br#"{"formatVersion":"0.10","units"x:{}}"#[..],
            &br#"{"formatVersion":"0.10","units"}"#[..],
            &br#"{"formatVersion":"0.10","units"  "#[..],
            &br#"{"formatVersion"x:"0.10"}"#[..],
        ] {
            match parse_traffic(input).expect_err("pre-value 失败") {
                ParseFailure::Syntax { path, position, .. } => {
                    assert_eq!(path, "$", "输入：{}", String::from_utf8_lossy(input));
                    assert_eq!(position, None);
                }
                other => panic!("expected Syntax, got {other:?}"),
            }
        }

        // 冒号已消费：value 阶段失败保持字段级（冒号后空白再 EOF）。
        match parse_traffic(br#"{"formatVersion":"0.10","units":  "#).expect_err("冒号后 EOF") {
            ParseFailure::Syntax { path, .. } => assert_eq!(path, "units"),
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    /// R3-8：非标量根 + trailing content 的 Data 锚只吃标量根 token（字符串
    /// 经 skip_string、true/false/null 定长、数字按 JSON 词法扫描），不吃进
    /// trailing content。
    #[test]
    fn scalar_root_error_anchor_excludes_trailing_content() {
        let (path, message, anchor) =
            shape_parts(parse_traffic(b"1 trailing").expect_err("数字根 + trailing"));
        assert_eq!(path, "$");
        assert_eq!(
            message,
            "invalid type: integer `1`, expected struct WireVersionHeader"
        );
        assert_eq!(anchor, ByteRange::new(0, 1), "锚只吃 `1`");

        let (_, message, anchor) =
            shape_parts(parse_traffic(br#""a" trailing"#).expect_err("字符串根 + trailing"));
        assert_eq!(
            message,
            "invalid type: string \"a\", expected struct WireVersionHeader"
        );
        assert_eq!(anchor, ByteRange::new(0, 3), "锚只吃 `\"a\"`");

        let (_, message, anchor) =
            shape_parts(parse_traffic(b"true trailing").expect_err("布尔根 + trailing"));
        assert_eq!(
            message,
            "invalid type: boolean `true`, expected struct WireVersionHeader"
        );
        assert_eq!(anchor, ByteRange::new(0, 4), "锚只吃 `true`");

        let (_, _message, anchor) =
            shape_parts(parse_traffic(b"1.5e+3 trailing").expect_err("指数数字根 + trailing"));
        assert_eq!(anchor, ByteRange::new(0, 6), "锚只吃 `1.5e+3`");
    }

    /// R4-2：数字根锚停在 serde 实际消费的 JSON number 词素边界——字符类
    /// 扫描会把 `1-2`/`1.2.3` 的 trailing 垃圾吃进锚（`json_number_end` 按
    /// `-? int frac? exp?` 语法推进）。
    #[test]
    fn numeric_root_error_anchor_stops_at_lexeme_end() {
        let (_, _, anchor) = shape_parts(parse_traffic(b"1-2").expect_err("数字根 + -2"));
        assert_eq!(anchor, ByteRange::new(0, 1), "锚只吃 `1`");

        let (_, _, anchor) = shape_parts(parse_traffic(b"1+2").expect_err("数字根 + +2"));
        assert_eq!(anchor, ByteRange::new(0, 1), "锚只吃 `1`");

        let (_, _, anchor) = shape_parts(parse_traffic(b"1.2.3").expect_err("数字根 + .3"));
        assert_eq!(anchor, ByteRange::new(0, 3), "锚只吃 `1.2`");

        let (_, _, anchor) = shape_parts(parse_traffic(b"-1.5e+3rest").expect_err("数字根 + rest"));
        assert_eq!(anchor, ByteRange::new(0, 7), "锚只吃 `-1.5e+3`");
    }

    /// R4-3：首个延迟失败确立后，后续根字段只捕获 token 不再物化 DTO（旧
    /// 两遍 loader 的 fail-fast 拒绝成本）；报错结果与跳过前逐字一致。
    #[test]
    fn first_deferred_failure_skips_later_field_materialization() {
        // `bogus` 在 units 之后、laneGraph 之前：units 正常解码，bogus 之
        // 后的全部字段（laneGraph…parking）跳过物化。
        let input = minimal_traffic().replacen(r#""laneGraph""#, r#""bogus": 1, "laneGraph""#, 1);
        crate::counters::reset();
        let (path, message, _anchor) =
            shape_parts(parse_traffic(input.as_bytes()).expect_err("未知根字段"));
        assert_eq!(path, "bogus");
        assert!(
            message.starts_with("unknown field `bogus`"),
            "消息：{message}"
        );
        // 失败前的 replay：formatVersion 标量 1 + units record 1 + 其两个
        // 标量字段各 1 = 4；bogus 之后 16 个字段的 token 被捕获但零 replay。
        let snapshot = crate::counters::snapshot();
        assert_eq!(snapshot.root_drivers, 1);
        assert_eq!(snapshot.replays, 4, "首个延迟失败后的字段不得 replay 物化");
    }

    /// R3 延迟 syntax 与延迟 shape 统一按锚点文档序裁决（与 extensions 内容
    /// 错误同一规则）：版本裁决先于一切内容错误；同序先到者胜。
    #[test]
    fn replay_syntax_defers_and_orders_by_document_position() {
        // 版本裁决优先于 replay syntax。
        let input = minimal_traffic()
            .replacen(
                r#""units": {"distance": "m", "time": "s"}"#,
                r#""units": ["m", "s", 0]"#,
                1,
            )
            .replacen(r#""0.10""#, r#""9.9""#, 1);
        match parse_traffic(input.as_bytes()).expect_err("不受支持的版本") {
            ParseFailure::UnsupportedVersion { actual, .. } => assert_eq!(actual, "9.9"),
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }

        // units syntax（文档序在前）先于 junctions shape。
        let input = minimal_traffic()
            .replacen(
                r#""units": {"distance": "m", "time": "s"}"#,
                r#""units": ["m", "s", 0]"#,
                1,
            )
            .replacen(r#""junctions": []"#, r#""junctions": [{"id": 1}]"#, 1);
        let failure = parse_traffic(input.as_bytes()).expect_err("syntax 先到者优先");
        match failure {
            ParseFailure::Syntax { path, .. } => assert_eq!(path, "units"),
            other => panic!("expected Syntax, got {other:?}"),
        }

        // junctions shape（文档序在前）先于 signals enum syntax。
        let input = minimal_traffic()
            .replacen(r#""junctions": []"#, r#""junctions": [{"id": 1}]"#, 1)
            .replacen(
                r#""signals": {"stopLines": [], "maneuverGates": [], "groups": [], "controllers": []}"#,
                r#""signals": {"stopLines": [{"id": "s", "edgeId": "e", "location": 1}], "maneuverGates": [], "groups": [], "controllers": []}"#,
                1,
            );
        let (path, message, _anchor) =
            shape_parts(parse_traffic(input.as_bytes()).expect_err("shape 先到者优先"));
        assert_eq!(path, "junctions[0].id");
        assert_eq!(message, "invalid type: integer `1`, expected a string");
    }
}
