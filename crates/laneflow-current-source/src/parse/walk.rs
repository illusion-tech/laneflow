//! 共享 visitor 游走层：record token 捕获、位置策略 hook、标量 replay 解码与
//! shape 候选机制。
//!
//! 机制（docs/design/current-package-import.md §7）：根文档由
//! `serde_json::Deserializer` 流式驱动；每个 field/element 的 value 先捕获为
//! `&RawValue` token（捕获即完成该子树的完整语法校验，且 token 借自原始输
//! 入，指针算术即得全局零基半开 byte 区间）；随后对 token 至多 replay 解码
//! 一次。token 必是合法 JSON；replay 内失败按 serde category 分流（R3：
//! 保留原生 category）——Data 归一为 shape 候选，Syntax 保留真实 serde
//! 错误为延迟 syntax（token 局部行列重建为全局位置 override，§7 :399-400），
//! 延迟 shape 与延迟 syntax 在版本裁决后按锚点文档序取先到者。

use std::borrow::Cow;
use std::fmt::{self, Write as _};

use serde::Deserialize;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde_json::error::Category;
use serde_json::value::RawValue;

use super::anchor::{ByteRange, skip_string};

/// visit_map/visit_seq 的中止信号：replay 失败（shape 或 syntax）经 `failure`
/// 外传，该消息永不浮出水面（凡 sentinel 出现处 `failure` 必为 `Some`）。
const SENTINEL: &str = "laneflow shape candidate sentinel";

/// 延迟 shape 候选：`path` 为规范 `$` 形式；`message` 为裸 serde 消息（无位
/// 置后缀）；`anchor` 为全局零基半开 byte 区间。
#[derive(Debug)]
pub(crate) struct ShapeCandidate {
    pub(crate) path: String,
    pub(crate) message: String,
    pub(crate) anchor: ByteRange,
}

/// replay/捕获失败通道（R3：保留原生 serde category）：Data 归一为 shape 候
/// 选；Syntax 保留真实 serde 错误为延迟 syntax。两者都延迟到版本裁决之后，
/// 按锚点文档序取先到者（`RootGate::defer_failure` / `first_deferred`）。
#[derive(Debug)]
pub(crate) enum ReplayFailure {
    /// Data 类失败（类型/缺字段/duplicate/unknown variant 等）的 shape 候选。
    Shape(ShapeCandidate),
    /// Syntax 类失败（number out of range、trailing characters、expected
    /// value、recursion limit 等）保留原生 category 的延迟 syntax。
    Syntax(DeferredSyntax),
}

impl From<ShapeCandidate> for ReplayFailure {
    fn from(candidate: ShapeCandidate) -> Self {
        Self::Shape(candidate)
    }
}

/// 位置记录策略：PR-3b 的位置表在相同 hook 点位收集（`CaptureLocations`），
/// 不重写 visitor；全部 hook 默认 no-op，泛型静态分派使 production 零成本。
pub(crate) trait LocationPolicy {
    /// 任意 field/element 的 value token（path 为该 field/element 的规范 path）。
    fn value_token(&mut self, _path: &str, _range: ByteRange) {}

    /// record（object）token。
    fn record_token(&mut self, _path: &str, _range: ByteRange) {}

    /// centerline point token。
    fn point_token(&mut self, _path: &str, _range: ByteRange) {}

    /// 根 object token 区间。
    fn root_object(&mut self, _path: &str, _range: ByteRange) {}
}

/// production 位置策略：全部 hook 为 no-op。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoLocations;

impl LocationPolicy for NoLocations {}

/// path 的规范形式：根（空串）为 `$`。
pub(crate) fn canonical(path: &str) -> &str {
    if path.is_empty() { "$" } else { path }
}

/// 解析上下文：原始输入、当前 JSON path 缓冲与位置策略。
pub(crate) struct Ctx<'de, L> {
    input: &'de [u8],
    path: String,
    locations: L,
}

impl<'de, L: LocationPolicy> Ctx<'de, L> {
    pub(crate) fn new(input: &'de [u8], locations: L) -> Self {
        Self {
            input,
            path: String::new(),
            locations,
        }
    }

    /// 当前 path 的规范形式（根为 `$`）。
    pub(crate) fn canonical_path(&self) -> &str {
        canonical(&self.path)
    }

    /// push 一个 object field：深度 1 无前导 `.`，与 serde_path_to_error 输出
    /// （`traffic.artifactRef`、`laneGraph.edges[0].speedLimit`）逐一对齐。
    pub(crate) fn push_field(&mut self, key: &str) -> usize {
        let mark = self.path.len();
        if mark > 0 {
            self.path.push('.');
        }
        self.path.push_str(key);
        mark
    }

    /// push 一个 array index：永远追加 `[i]`。
    pub(crate) fn push_index(&mut self, index: usize) -> usize {
        let mark = self.path.len();
        write!(self.path, "[{index}]").expect("path 写入 String 不可失败");
        mark
    }

    pub(crate) fn truncate(&mut self, mark: usize) {
        self.path.truncate(mark);
    }

    /// 以当前 path（field/element 级）构造候选。
    pub(crate) fn candidate(&self, message: String, anchor: ByteRange) -> ShapeCandidate {
        ShapeCandidate {
            path: self.canonical_path().to_owned(),
            message,
            anchor,
        }
    }

    /// 以 `mark` 截断处 path（record 级）构造候选。
    pub(crate) fn candidate_at(
        &self,
        mark: usize,
        message: String,
        anchor: ByteRange,
    ) -> ShapeCandidate {
        ShapeCandidate {
            path: canonical(&self.path[..mark]).to_owned(),
            message,
            anchor,
        }
    }

    /// 以当前 path 构造 shape 类 replay 失败（`Err(ctx.failure(...))` 直返点
    /// 免 `.into()`；`ok_or_else` 点经 `?` 的 `From` 自动转换仍用 `candidate`）。
    pub(crate) fn failure(&self, message: String, anchor: ByteRange) -> ReplayFailure {
        ReplayFailure::Shape(self.candidate(message, anchor))
    }

    /// token 在原始输入中的全局零基半开区间（token 借自输入，指针算术）。
    pub(crate) fn token_range(&self, token: &RawValue) -> ByteRange {
        let base = self.input.as_ptr() as usize;
        let start = token.get().as_bytes().as_ptr() as usize - base;
        let end = start + token.get().len();
        ByteRange::new(
            u32::try_from(start).unwrap_or(u32::MAX),
            u32::try_from(end).unwrap_or(u32::MAX),
        )
    }

    pub(crate) fn policy_value(&mut self, range: ByteRange) {
        let path = canonical(&self.path);
        self.locations.value_token(path, range);
    }

    pub(crate) fn policy_record(&mut self, range: ByteRange) {
        let path = canonical(&self.path);
        self.locations.record_token(path, range);
    }

    pub(crate) fn policy_point(&mut self, range: ByteRange) {
        let path = canonical(&self.path);
        self.locations.point_token(path, range);
    }

    pub(crate) fn policy_root(&mut self, range: ByteRange) {
        let path = canonical(&self.path);
        self.locations.root_object(path, range);
    }
}

/// struct 字段元数据：声明名 + 是否有 `#[serde(default)]`（R2 T3：位置序列
/// 元素耗尽时，带 default 的剩余字段取默认值而非 invalid length）。元数据与
/// 旧 derive DTO（fe41706 wire.rs）逐字段盘点一致：extensions / extendsId /
/// laneGroupId / timeWindows / regulation / priority / source / areaId 共 8 个。
#[derive(Clone, Copy)]
pub(crate) struct FieldSpec {
    name: &'static str,
    has_default: bool,
}

/// 必填字段（无 `#[serde(default)]`）。
pub(crate) const fn req(name: &'static str) -> FieldSpec {
    FieldSpec {
        name,
        has_default: false,
    }
}

/// `#[serde(default)]` 字段。
pub(crate) const fn dflt(name: &'static str) -> FieldSpec {
    FieldSpec {
        name,
        has_default: true,
    }
}

/// object 的 DeserializeSeed/Visitor 合体：捕获 value token、打位置 hook、调
/// handler（参数 `(ctx, key, value token, value 区间, push 前 path 标记)`）；
/// handler 失败时存候选并以 sentinel 中止。`container_start` 为容器 `{` 的
/// 全局 byte 起点，只在 pre-value 失败时重扫冒号位（R5，error-only 路径）。
pub(crate) struct ObjectSeed<'a, 'de, L, F> {
    ctx: &'a mut Ctx<'de, L>,
    failure: &'a mut Option<ReplayFailure>,
    expecting: &'static str,
    container_start: u32,
    handler: F,
}

impl<'de, L, F> DeserializeSeed<'de> for ObjectSeed<'_, 'de, L, F>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ReplayFailure>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(self)
    }
}

impl<'de, L, F> Visitor<'de> for ObjectSeed<'_, 'de, L, F>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ReplayFailure>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.expecting)
    }

    fn visit_map<A>(mut self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        // 上一 value token 的全局终点（首字段为容器 `{` 之后），只在
        // next_value 失败时用于重扫冒号位（R5，error-only 路径）。
        let mut prev_end = self.container_start as usize + 1;
        // key 解为 Cow：无转义仍为借用（零开销）；含 `\uXXXX` 等转义的合法 key
        // 解码为 owned 后与明文 key 走同一路径（derive 行为平价）。
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            let key = key.as_ref();
            let mark = self.ctx.push_field(key);
            let value = match map.next_value::<&RawValue>() {
                Ok(value) => value,
                Err(error) => {
                    // R5：冒号/分隔符阶段（pre-value）失败归 record 级
                    // path——serde_path_to_error 的 MapAccess::next_value_seed
                    // 在 delegate 失败时以 parent chain 触发（key 段落只随
                    // TrackedSeed 进入 value 解析）；冒号已消费 ⇒ 失败在
                    // value 阶段 ⇒ 保持字段级（R3-3 dispute pin 同案例）。
                    if !colon_consumed(self.ctx.input, prev_end) {
                        self.ctx.truncate(mark);
                    }
                    return Err(error);
                }
            };
            let range = self.ctx.token_range(value);
            self.ctx.policy_value(range);
            match (self.handler)(self.ctx, key, value, range, mark) {
                Ok(()) => {
                    prev_end = range.end as usize;
                    self.ctx.truncate(mark);
                }
                Err(candidate) => {
                    *self.failure = Some(candidate);
                    return Err(de::Error::custom(SENTINEL));
                }
            }
        }
        Ok(())
    }
}

/// R5：object 当前 key 的冒号是否已（将被 serde）消费——从上一 value token
/// 终点重扫：`ws / "," / key 字符串词素 / ws` 后下一 byte 为 `:` 即冒号可
/// 消费（失败进入 value 解析，path 归字段级）；否则为冒号/分隔符阶段失败
/// （path 归 record 级，serde_path_to_error 的 parent chain 触发语义，探针
/// 实证 `de.rs:1531-1533`）。扫描失步防御性归 record 级（与 path_to_error
/// 的保守 parent 回退一致）。只在 pre-value 失败路径调用，热路径零开销。
fn colon_consumed(input: &[u8], from: usize) -> bool {
    fn skip_ws(input: &[u8], mut index: usize) -> usize {
        while index < input.len() && matches!(input[index], b' ' | b'\t' | b'\n' | b'\r') {
            index += 1;
        }
        index
    }

    let mut index = skip_ws(input, from);
    if input.get(index) == Some(&b',') {
        index = skip_ws(input, index + 1);
    }
    if input.get(index) != Some(&b'"') {
        return false;
    }
    index = skip_ws(input, skip_string(input, index));
    input.get(index) == Some(&b':')
}

/// struct 位置序列（seq-form）的 DeserializeSeed/Visitor 合体：按声明序逐位
/// 置把 `fields[index]` 作为 key 调同一个 per-field handler（元素级 path 用
/// `[index]` 段落，与 serde_path_to_error 对 seq-form 的输出一致）。元素在
/// 索引 i 耗尽后：`#[serde(default)]` 字段跳过（slot 保持未设，按 map-form
/// 缺席处理）；第一个无 default 的剩余字段 j 以 derive 的 invalid length 文
/// 本产出候选（path 停在 record 级——serde_path_to_error 探针实证，R2 T3）。
/// 多余元素不消费，由 serde_json 的 `end_seq` 报 `trailing characters`（与
/// derive 一致）。
pub(crate) struct StructSeqSeed<'a, 'de, L, F> {
    ctx: &'a mut Ctx<'de, L>,
    failure: &'a mut Option<ReplayFailure>,
    expecting: &'static str,
    fields: &'static [FieldSpec],
    anchor: ByteRange,
    handler: F,
}

impl<'de, L, F> DeserializeSeed<'de> for StructSeqSeed<'_, 'de, L, F>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ReplayFailure>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, L, F> Visitor<'de> for StructSeqSeed<'_, 'de, L, F>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ReplayFailure>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.expecting)
    }

    fn visit_seq<A>(mut self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        for (index, spec) in self.fields.iter().enumerate() {
            let Some(element) = seq.next_element::<&RawValue>()? else {
                // 元素耗尽：剩余字段中第一个无 `#[serde(default)]` 的字段 j 报
                // derive invalid length（path 停在 record 级）；全部带 default
                // 则取默认值（slot 保持未设，按 map-form 缺席处理）。
                let missing = self.fields[index..]
                    .iter()
                    .position(|spec| !spec.has_default)
                    .map(|offset| index + offset);
                let Some(missing_index) = missing else {
                    return Ok(());
                };
                let message =
                    invalid_length_message(missing_index, self.expecting, self.fields.len());
                *self.failure = Some(self.ctx.candidate(message, self.anchor).into());
                return Err(de::Error::custom(SENTINEL));
            };
            let mark = self.ctx.push_index(index);
            let range = self.ctx.token_range(element);
            self.ctx.policy_value(range);
            match (self.handler)(self.ctx, spec.name, element, range, mark) {
                Ok(()) => self.ctx.truncate(mark),
                Err(candidate) => {
                    *self.failure = Some(candidate);
                    return Err(de::Error::custom(SENTINEL));
                }
            }
        }
        Ok(())
    }
}

/// array 的 DeserializeSeed/Visitor 合体；handler 参数为 `(ctx, index,
/// element token, element 区间)`。
pub(crate) struct ArraySeed<'a, 'de, L, F> {
    ctx: &'a mut Ctx<'de, L>,
    failure: &'a mut Option<ReplayFailure>,
    expecting: &'static str,
    handler: F,
}

impl<'de, L, F> DeserializeSeed<'de> for ArraySeed<'_, 'de, L, F>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, usize, &'de RawValue, ByteRange) -> Result<(), ReplayFailure>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, L, F> Visitor<'de> for ArraySeed<'_, 'de, L, F>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, usize, &'de RawValue, ByteRange) -> Result<(), ReplayFailure>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.expecting)
    }

    fn visit_seq<A>(mut self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut index = 0_usize;
        while let Some(element) = seq.next_element::<&RawValue>()? {
            let mark = self.ctx.push_index(index);
            let range = self.ctx.token_range(element);
            self.ctx.policy_value(range);
            match (self.handler)(self.ctx, index, element, range) {
                Ok(()) => self.ctx.truncate(mark),
                Err(candidate) => {
                    *self.failure = Some(candidate);
                    return Err(de::Error::custom(SENTINEL));
                }
            }
            index += 1;
        }
        Ok(())
    }
}

/// 驱动一次 object walk（根驱动与 replay 共用）。返回 `Err(serde_json::Error)`
/// 表示真实 serde 错误；shape 候选经 `failure` 外传（此时返回值必为 sentinel
/// 包装的 `Err`）。`container_start` 为容器 `{` 的全局 byte 起点（R5 冒号
/// 位重扫，只在 pre-value 失败路径使用；replay 的 token 已校验语法，该路径
/// 不可达）。
pub(crate) fn drive_object<'de, L, F, D>(
    ctx: &mut Ctx<'de, L>,
    failure: &mut Option<ReplayFailure>,
    expecting: &'static str,
    container_start: u32,
    deserializer: D,
    handler: F,
) -> Result<(), serde_json::Error>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ReplayFailure>,
    D: serde::Deserializer<'de, Error = serde_json::Error>,
{
    ObjectSeed {
        ctx,
        failure,
        expecting,
        container_start,
        handler,
    }
    .deserialize(deserializer)
}

/// 驱动一次 struct seq-form walk（与 `drive_object` 对称；`anchor` 为缺位
/// 候选的所属 record/root token 区间）。
pub(crate) fn drive_seq<'de, L, F, D>(
    ctx: &mut Ctx<'de, L>,
    failure: &mut Option<ReplayFailure>,
    expecting: &'static str,
    fields: &'static [FieldSpec],
    anchor: ByteRange,
    deserializer: D,
    handler: F,
) -> Result<(), serde_json::Error>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ReplayFailure>,
    D: serde::Deserializer<'de, Error = serde_json::Error>,
{
    StructSeqSeed {
        ctx,
        failure,
        expecting,
        fields,
        anchor,
        handler,
    }
    .deserialize(deserializer)
}

#[inline]
fn count_replay(range: ByteRange) {
    #[cfg(debug_assertions)]
    crate::counters::record_replay(range.start);
    #[cfg(not(debug_assertions))]
    let _ = range;
}

/// replay serde 错误分流（R3：保留原生 category）：Syntax 保留真实 serde 错
/// 误为延迟 syntax（token 局部行列 → 相对 byte offset → 加 token 全局起点
/// 重建全局位置 override，SSOT :399-400；span 用 override，payload 内部位置
/// 保持 token 局部）；Data 以容器 token 为锚归一为 shape 候选（Eof/Io 在合
/// 法 token 内不可达，防御性随 Data 归一）。
fn classify_replay_error<L>(
    ctx: &Ctx<'_, L>,
    token: &RawValue,
    range: ByteRange,
    error: serde_json::Error,
) -> ReplayFailure
where
    L: LocationPolicy,
{
    match error.classify() {
        Category::Syntax => ReplayFailure::Syntax(DeferredSyntax {
            path: ctx.canonical_path().to_owned(),
            position: global_position(ctx.input, range.start, token.get().as_bytes(), &error),
            token_start: range.start,
            source: error,
        }),
        _ => ReplayFailure::Shape(ctx.candidate(strip_position_suffix(error.to_string()), range)),
    }
}

/// replay 解码 record token：handler 失败直接外传；真实 serde 错误按
/// `classify_replay_error` 分流（Syntax 保留原生 category，Data 归一候选）。
/// token 形态分派：`{` 走 map，`[` 按 `fields` 声明序走位置序列（derive
/// struct seq-form 平价），其余形态由 map 路径报 invalid type。
pub(crate) fn decode_record<'de, L, F>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    expecting: &'static str,
    fields: &'static [FieldSpec],
    handler: F,
) -> Result<(), ReplayFailure>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ReplayFailure>,
{
    ctx.policy_record(range);
    count_replay(range);
    let mut failure = None;
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    let result = if token.get().trim_start().starts_with('[') {
        drive_seq(
            ctx,
            &mut failure,
            expecting,
            fields,
            range,
            &mut deserializer,
            handler,
        )
    } else {
        drive_object(
            ctx,
            &mut failure,
            expecting,
            range.start,
            &mut deserializer,
            handler,
        )
    };
    if let Some(failure) = failure {
        return Err(failure);
    }
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(classify_replay_error(ctx, token, range, error)),
    }
}

/// untagged 分派专用：完整扫描 record（handler 只记录、不报错）；`Ok(true)`
/// 表示结构干净，`Ok(false)` 表示出现真实 serde 错误（由调用方归一化为
/// mismatch 候选——旧 untagged derive 对任何内部错误都报 Data mismatch，故
/// 此处不区分 category）。handler 主动产出的失败仍按 `Err` 传播。
pub(crate) fn scan_record<'de, L, F>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    expecting: &'static str,
    handler: F,
) -> Result<bool, ReplayFailure>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ReplayFailure>,
{
    ctx.policy_record(range);
    count_replay(range);
    let mut failure = None;
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    let result = drive_object(
        ctx,
        &mut failure,
        expecting,
        range.start,
        &mut deserializer,
        handler,
    );
    if let Some(failure) = failure {
        return Err(failure);
    }
    Ok(result.is_ok())
}

/// replay 解码 array token（Vec 与定长数组共用）；语义同 `decode_record`。
pub(crate) fn decode_array<'de, L, F>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    expecting: &'static str,
    handler: F,
) -> Result<(), ReplayFailure>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, usize, &'de RawValue, ByteRange) -> Result<(), ReplayFailure>,
{
    count_replay(range);
    let mut failure = None;
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    let result = ArraySeed {
        ctx: &mut *ctx,
        failure: &mut failure,
        expecting,
        handler,
    }
    .deserialize(&mut deserializer);
    if let Some(failure) = failure {
        return Err(failure);
    }
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(classify_replay_error(ctx, token, range, error)),
    }
}

/// replay 解码标量/透明值 token（String、u32、u64、f64、Vec<Value> 等）；失
/// 败按 `classify_replay_error` 分流——Syntax（如 `1e999` 的 `number out of
/// range`）保留原生 category 为延迟 syntax，Data 归一为 shape 候选。
pub(crate) fn decode_scalar<'de, T, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<T, ReplayFailure>
where
    T: Deserialize<'de>,
    L: LocationPolicy,
{
    count_replay(range);
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    T::deserialize(&mut deserializer)
        .map_err(|error| classify_replay_error(ctx, token, range, error))
}

/// 字符串枚举 token：经 `deserialize_enum` 复刻 derive 的 visitor 流转（字符串
/// 经 `visit_str` 查表报 `unknown variant`（Data → shape 候选）；非字符串/非
/// `{` 标量由 serde_json 报真实 `expected value`（Syntax → 延迟 syntax，R3-4
/// 保留原生 category）），expecting 文本 `enum {name}` 与 derive 逐字节一致。
pub(crate) fn decode_enum<'de, T, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    name: &'static str,
    variants: &'static [&'static str],
    table: &'static [(&'static str, T)],
) -> Result<T, ReplayFailure>
where
    T: Copy + 'static,
    L: LocationPolicy,
{
    count_replay(range);
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    EnumSeed {
        name,
        variants,
        table,
    }
    .deserialize(&mut deserializer)
    .map_err(|error| classify_replay_error(ctx, token, range, error))
}

struct EnumSeed<T: 'static> {
    name: &'static str,
    variants: &'static [&'static str],
    table: &'static [(&'static str, T)],
}

impl<'de, T: Copy + 'static> DeserializeSeed<'de> for EnumSeed<T> {
    type Value = T;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_enum(self.name, self.variants, self)
    }
}

impl<'de, T: Copy + 'static> Visitor<'de> for EnumSeed<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "enum {}", self.name)
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: de::EnumAccess<'de>,
    {
        let (value, variant) = data.variant_seed(VariantSeed {
            variants: self.variants,
            table: self.table,
        })?;
        variant.unit_variant()?;
        Ok(value)
    }
}

struct VariantSeed<T: 'static> {
    variants: &'static [&'static str],
    table: &'static [(&'static str, T)],
}

impl<'de, T: Copy + 'static> DeserializeSeed<'de> for VariantSeed<T> {
    type Value = T;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(self)
    }
}

impl<'de, T: Copy + 'static> Visitor<'de> for VariantSeed<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("variant identifier")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        match self.table.iter().find(|(name, _)| *name == value) {
            Some((_, variant)) => Ok(*variant),
            None => Err(E::unknown_variant(value, self.variants)),
        }
    }
}

/// 显式 `null` 拒绝（`non_null_option` 的 visitor 形态）：缺省由调用方按
/// `None` 处理；消息与旧 `deserialize_with` helper 逐字节一致。
pub(crate) const NON_NULL_MESSAGE: &str = "可选字段不接受显式 null；请省略该字段";

pub(crate) fn reject_explicit_null<L>(
    ctx: &Ctx<'_, L>,
    token: &RawValue,
    range: ByteRange,
) -> Result<(), ReplayFailure>
where
    L: LocationPolicy,
{
    if token.get().trim() == "null" {
        return Err(ctx.failure(NON_NULL_MESSAGE.to_owned(), range));
    }
    Ok(())
}

/// 拒绝显式 null 的 Option<String> 字段（extendsId/laneGroupId/source）。
pub(crate) fn decode_non_null_string<'de, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<String, ReplayFailure>
where
    L: LocationPolicy,
{
    reject_explicit_null(ctx, token, range)?;
    decode_scalar(ctx, token, range)
}

/// priority 原始数值字面量：只做 JSON number 词法检查，不经浮点转换；显式
/// `null` 与非数值字面量的消息与旧 `access_priority_lexeme` 逐字节一致。
pub(crate) fn decode_priority<L>(
    ctx: &Ctx<'_, L>,
    token: &RawValue,
    range: ByteRange,
) -> Result<String, ReplayFailure>
where
    L: LocationPolicy,
{
    let lexeme = token.get().trim();
    if lexeme == "null" {
        return Err(ctx.failure(NON_NULL_MESSAGE.to_owned(), range));
    }
    if is_json_number_lexeme(lexeme) {
        Ok(lexeme.to_owned())
    } else {
        Err(ctx.failure(
            format!("priority 必须是 JSON number，实际为 `{lexeme}`"),
            range,
        ))
    }
}

/// JSON number 语法：`-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`。
fn is_json_number_lexeme(lexeme: &str) -> bool {
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    let lexeme = lexeme.strip_prefix('-').unwrap_or(lexeme);
    let (mantissa, exponent) = match lexeme.find(['e', 'E']) {
        Some(index) => (&lexeme[..index], Some(&lexeme[index + 1..])),
        None => (lexeme, None),
    };
    if let Some(exponent) = exponent {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if !digits(exponent) {
            return false;
        }
    }
    let (integer, fraction) = match mantissa.find('.') {
        Some(index) => (&mantissa[..index], Some(&mantissa[index + 1..])),
        None => (mantissa, None),
    };
    let integer_ok = integer == "0"
        || (integer.starts_with(|c: char| c.is_ascii_digit() && c != '0') && digits(integer));
    integer_ok && fraction.is_none_or(digits)
}

/// timeWindows 不透明 presence：显式 `null` 拒绝；只校验 JSON type 是数组，
/// 窗口内容一律不解码（capability guard 先于 shape）。
pub(crate) fn decode_time_windows<'de, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<bool, ReplayFailure>
where
    L: LocationPolicy,
{
    reject_explicit_null(ctx, token, range)?;
    if token.get().trim_start().starts_with('[') {
        return Ok(true);
    }
    // 非数组 token：借用 serde 的 invalid type 消息（`expected a sequence`）。
    decode_scalar::<Vec<serde_json::Value>, L>(ctx, token, range).map(|_| true)
}

/// 延迟到版本裁决之后的 syntax 失败：path、真实 serde 错误（token 局部位
/// 置）与重建的全局一基位置（span override）。来源：extensions 内容校验
/// （R2 T5）与 replay/捕获内 Syntax 类失败（R3：保留原生 serde category）。
#[derive(Debug)]
pub(crate) struct DeferredSyntax {
    pub(crate) path: String,
    pub(crate) source: serde_json::Error,
    /// 全局一基 (line, column)（span 用；payload 内部位置保持 token 局部）。
    pub(crate) position: (u32, u32),
    /// 文档序比较键：失败 token 的全局起点。
    pub(crate) token_start: u32,
}

/// 根 `extensions`：非 object 借用 serde 的 invalid type / number range 错误
/// （经 `classify_replay_error` 分流）；object 内容以 sink visitor 单遍校验
/// （SSOT §7：禁 Value/Content 树，token 只驱动一遍）。零拷贝（R3-5）：sink
/// 直接在借用 token 切片上驱动，无合成 wrapper 分配与拷贝；数值 range 由
/// serde_json 自身的 u64/i64/f64 解析执行（`1e999` → `number out of
/// range`）。递归深度由 sink 自计数执行（`EXTENSIONS_CONTENT_DEPTH_LIMIT`）：
/// 旧全量解析中 extensions object 是文档第 2 层容器，serde_json 128 层预算
/// 在第 126 个内容容器报 `recursion limit exceeded`；token 直达驱动少了文
/// 档根一层（serde 预算等效放宽一层），故 sink 在进入第 126 个内容容器时
/// 中止，真实 recursion 错误由定长合成探针捕获，位置由边界配对扫描重建为
/// 全局 override。
pub(crate) fn check_extensions<'de, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<(), ReplayFailure>
where
    L: LocationPolicy,
{
    if !token.get().trim_start().starts_with('{') {
        return decode_scalar::<serde_json::Map<String, serde_json::Value>, L>(ctx, token, range)
            .map(|_| ());
    }
    count_replay(range);
    let mut suffix = String::new();
    let mut depth_exceeded = false;
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    let result = ExtensionsMapSeed {
        suffix: &mut suffix,
        depth_exceeded: &mut depth_exceeded,
    }
    .deserialize(&mut deserializer);
    match result {
        Ok(()) => Ok(()),
        // sink 自计数中止（旧 126 层边界）：真实 recursion 错误由合成探针捕
        // 获，全局位置取第 126 个内容容器的开括号（旧 peek_error 的同一点位）。
        Err(_) if depth_exceeded => Err(ReplayFailure::Syntax(DeferredSyntax {
            path: format!("extensions{suffix}"),
            position: position_of_offset(
                ctx.input,
                range.start as usize + content_depth_boundary_offset(token.get().as_bytes()),
            ),
            token_start: range.start,
            source: harvest_recursion_error(),
        })),
        // 内容 syntax 失败（数值 range 等）：path 深入 extensions 子树，token
        // 局部位置重建为全局 override（Data 在合法 token 内不可达，防御归一）。
        Err(source) => Err(match source.classify() {
            Category::Syntax => ReplayFailure::Syntax(DeferredSyntax {
                path: format!("extensions{suffix}"),
                position: global_position(ctx.input, range.start, token.get().as_bytes(), &source),
                token_start: range.start,
                source,
            }),
            _ => ReplayFailure::Shape(ShapeCandidate {
                path: format!("extensions{suffix}"),
                message: strip_position_suffix(source.to_string()),
                anchor: range,
            }),
        }),
    }
}

/// extensions 内容容器深度上限（R3-5 零拷贝平价）：旧全量解析中 extensions
/// object 是文档第 2 层容器，serde_json 128 层预算在进入第 128 层容器时报
/// `recursion limit exceeded`——即第 126 个内容容器（旧探针：125 过/126 拒）。
const EXTENSIONS_CONTENT_DEPTH_LIMIT: u8 = 126;

/// sink 自计数中止信号（永不浮出水面；出现处 `depth_exceeded` 必为 true）。
const DEPTH_SENTINEL: &str = "laneflow extensions depth sentinel";

/// 定长合成探针捕获 serde_json 真实的 `recursion limit exceeded`（Syntax
/// category 与核心消息；只在深度中止的错误路径调用，常量成本）。payload
/// 内部位置为探针局部坐标，不是位置事实源——span 由边界扫描的全局
/// override 承载（SSOT :445 不冻结 line/column 数值，R3-2 文档契约）。
fn harvest_recursion_error() -> serde_json::Error {
    let mut probe = String::with_capacity(257);
    for _ in 0..128 {
        probe.push('[');
    }
    probe.push('1');
    for _ in 0..128 {
        probe.push(']');
    }
    serde_json::from_slice::<serde_json::Value>(probe.as_bytes())
        .expect_err("合成探针必然触发 serde_json 递归上限")
}

/// 配对扫描求第 `EXTENSIONS_CONTENT_DEPTH_LIMIT` 个内容容器（旧递归边界）
/// 在 extensions token 内的 byte offset（字符串与转义整体跳过）；只在 sink
/// 中止后的错误路径调用。
fn content_depth_boundary_offset(token: &[u8]) -> usize {
    let mut depth = 0_u32;
    let mut index = 0_usize;
    while index < token.len() {
        match token[index] {
            b'"' => {
                index = skip_string(token, index);
                continue;
            }
            b'{' | b'[' => {
                depth += 1;
                // extensions object 本身是第 1 层；第 126 个内容容器是第 127 层。
                if depth == u32::from(EXTENSIONS_CONTENT_DEPTH_LIMIT) + 1 {
                    return index;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }
    // 不可达（sink 已在第 126 个内容容器处中止）；防御性收尾。
    token.len()
}

/// 把 token 局部 serde 位置重建为全局一基位置：局部 (line,column) → token
/// 内 byte offset → 加 token 全局起点后做 allocation-free 前缀扫描
/// （SSOT :399-400）。
fn global_position(
    input: &[u8],
    token_start: u32,
    token: &[u8],
    source: &serde_json::Error,
) -> (u32, u32) {
    let local_offset = offset_of_position(token, source.line(), source.column());
    position_of_offset(input, token_start as usize + local_offset)
}

/// 一基 (line,column) → 零基 byte offset（line 按 LF、column 按 byte，与
/// anchor::range_span 的计数规则一致；越界防御性收尾到末尾）。
fn offset_of_position(bytes: &[u8], line: usize, column: usize) -> usize {
    let mut current_line = 1_usize;
    let mut current_column = 1_usize;
    for (index, byte) in bytes.iter().enumerate() {
        if current_line == line && current_column == column {
            return index;
        }
        if *byte == b'\n' {
            current_line += 1;
            current_column = 1;
        } else {
            current_column += 1;
        }
    }
    bytes.len()
}

/// 零基 byte offset → 一基 (line,column)。
fn position_of_offset(input: &[u8], offset: usize) -> (u32, u32) {
    let end = offset.min(input.len());
    let mut line = 1_u32;
    let mut column = 1_u32;
    for byte in &input[..end] {
        if *byte == b'\n' {
            line = line.saturating_add(1);
            column = 1;
        } else {
            column = column.saturating_add(1);
        }
    }
    (line, column)
}

/// extensions object 顶层 seed：`deserialize_map` 直接驱动借用 token（零拷
/// 贝，R3-5；expecting `a map`，与旧 `Map<String, Value>` 的 invalid type
/// 文本一致；token 已预检 `{`）。
struct ExtensionsMapSeed<'a> {
    suffix: &'a mut String,
    depth_exceeded: &'a mut bool,
}

impl<'de> serde::de::DeserializeSeed<'de> for ExtensionsMapSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ExtensionsSink {
            suffix: self.suffix,
            depth: 0,
            depth_exceeded: self.depth_exceeded,
        })
    }
}

/// extensions 内容的 sink visitor：不物化任何值（所有标量丢弃）；object/
/// array 递归经 `deserialize_any` 驱动，path 后缀（`.key`/`[index]`）在成功
/// 返回时截断、失败传播时保留在失败深度（serde_path_to_error 文本平价）。
/// `depth` 为内容容器自计数（extensions object 自身为 0，标量不计），进入
/// 第 `EXTENSIONS_CONTENT_DEPTH_LIMIT` 个内容容器时置 `depth_exceeded` 并
/// 以 `DEPTH_SENTINEL` 中止（零拷贝递归边界，R3-5）。
struct ExtensionsSink<'a> {
    suffix: &'a mut String,
    depth: u8,
    depth_exceeded: &'a mut bool,
}

impl<'de> serde::de::DeserializeSeed<'de> for ExtensionsSink<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for ExtensionsSink<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a map")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.depth == EXTENSIONS_CONTENT_DEPTH_LIMIT {
            *self.depth_exceeded = true;
            return Err(de::Error::custom(DEPTH_SENTINEL));
        }
        let mut index = 0_usize;
        loop {
            let mark = self.suffix.len();
            self.suffix.push_str(&format!("[{index}]"));
            let suffix = &mut *self.suffix;
            let depth = self.depth.saturating_add(1);
            let depth_exceeded = &mut *self.depth_exceeded;
            match seq.next_element_seed(ExtensionsSink {
                suffix,
                depth,
                depth_exceeded,
            }) {
                Ok(Some(())) => {
                    self.suffix.truncate(mark);
                    index += 1;
                }
                Ok(None) => {
                    self.suffix.truncate(mark);
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.depth == EXTENSIONS_CONTENT_DEPTH_LIMIT {
            *self.depth_exceeded = true;
            return Err(de::Error::custom(DEPTH_SENTINEL));
        }
        while let Some(key) = map.next_key::<String>()? {
            let mark = self.suffix.len();
            self.suffix.push('.');
            self.suffix.push_str(&key);
            let suffix = &mut *self.suffix;
            let depth = self.depth.saturating_add(1);
            let depth_exceeded = &mut *self.depth_exceeded;
            match map.next_value_seed(ExtensionsSink {
                suffix,
                depth,
                depth_exceeded,
            }) {
                Ok(()) => self.suffix.truncate(mark),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

/// centerline point `[f64; 3]`：读恰好 3 轴（轴级 path/锚），复刻 serde 定
/// 长数组行为（R3-7）——第 4 元素不消费，由 serde_json 在 seq visitor 返回
/// 后报真实 `trailing characters`（Syntax，经 `classify_replay_error` 保留
/// 原生 category）；不足 3 轴报 serde 定长数组的 invalid length shape 候选
/// （锚=point token）。
pub(crate) fn decode_point<'de, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<[f64; 3], ReplayFailure>
where
    L: LocationPolicy,
{
    ctx.policy_point(range);
    count_replay(range);
    let mut failure = None;
    let mut axes = [0.0_f64; 3];
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    let result = PointSeed {
        ctx: &mut *ctx,
        failure: &mut failure,
        anchor: range,
        axes: &mut axes,
    }
    .deserialize(&mut deserializer);
    if let Some(failure) = failure {
        return Err(failure);
    }
    match result {
        Ok(()) => Ok(axes),
        Err(error) => Err(classify_replay_error(ctx, token, range, error)),
    }
}

/// `[f64; 3]` 的 seed/visitor：`deserialize_tuple(3, _)` 与 serde 定长数组
/// impl 同一入口；visit_seq 读恰好 3 轴即返回（多余元素留给 serde_json 的
/// seq 收尾检查报 `trailing characters`）。
struct PointSeed<'a, 'de, L> {
    ctx: &'a mut Ctx<'de, L>,
    failure: &'a mut Option<ReplayFailure>,
    anchor: ByteRange,
    axes: &'a mut [f64; 3],
}

impl<'de, L: LocationPolicy> DeserializeSeed<'de> for PointSeed<'_, 'de, L> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_tuple(3, self)
    }
}

impl<'de, L: LocationPolicy> Visitor<'de> for PointSeed<'_, 'de, L> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an array of length 3")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        for index in 0..3_usize {
            let Some(element) = seq.next_element::<&RawValue>()? else {
                // 不足 3 轴：serde 定长数组的 invalid length（Data → shape 候
                // 选，锚=point token，path 停在 point 级）。
                let message = format!("invalid length {index}, expected an array of length 3");
                *self.failure = Some(self.ctx.candidate(message, self.anchor).into());
                return Err(de::Error::custom(SENTINEL));
            };
            let mark = self.ctx.push_index(index);
            let element_range = self.ctx.token_range(element);
            self.ctx.policy_value(element_range);
            match decode_scalar::<f64, L>(self.ctx, element, element_range) {
                Ok(axis) => {
                    self.axes[index] = axis;
                    self.ctx.truncate(mark);
                }
                Err(failure) => {
                    *self.failure = Some(failure);
                    return Err(de::Error::custom(SENTINEL));
                }
            }
        }
        Ok(())
    }
}

/// 单赋值槽：第二次 occurrence 报 duplicate field（serde_path_to_error 实证：
/// path 为所属 record 级），随后按 `decode` 解码 value token。
pub(crate) fn set_once<'de, T, L, F>(
    ctx: &mut Ctx<'de, L>,
    slot: &mut Option<T>,
    key: &'static str,
    value: &'de RawValue,
    range: ByteRange,
    mark: usize,
    decode: F,
) -> Result<(), ReplayFailure>
where
    L: LocationPolicy,
    F: FnOnce(&mut Ctx<'de, L>, &'de RawValue, ByteRange) -> Result<T, ReplayFailure>,
{
    if slot.is_some() {
        return Err(ctx
            .candidate_at(mark, duplicate_field_message(key), range)
            .into());
    }
    *slot = Some(decode(ctx, value, range)?);
    Ok(())
}

/// `missing field` 消息（与 serde derive 逐字节一致）。
pub(crate) fn missing_field_message(field: &'static str) -> String {
    <serde_json::Error as de::Error>::missing_field(field).to_string()
}

/// `duplicate field` 消息（与 serde derive 逐字节一致）。
pub(crate) fn duplicate_field_message(field: &'static str) -> String {
    <serde_json::Error as de::Error>::duplicate_field(field).to_string()
}

/// `unknown field` 消息（serde_core `unknown_field`/`OneOf` 的逐字节复刻：1
/// 个 `` `a` ``，2 个 `` `a` or `b` ``，≥3 个 `one of `a`, `b`, `c``）。
pub(crate) fn unknown_field_message(field: &str, expected: &[FieldSpec]) -> String {
    if expected.is_empty() {
        return format!("unknown field `{field}`, there are no fields");
    }
    let mut list = String::new();
    if expected.len() == 1 {
        list.push_str(&format!("`{}`", expected[0].name));
    } else if expected.len() == 2 {
        list.push_str(&format!("`{}` or `{}`", expected[0].name, expected[1].name));
    } else {
        list.push_str("one of ");
        for (index, spec) in expected.iter().enumerate() {
            if index > 0 {
                list.push_str(", ");
            }
            list.push_str(&format!("`{}`", spec.name));
        }
    }
    format!("unknown field `{field}`, expected {list}")
}

/// 位置序列缺位的 derive `invalid length` 消息（`struct X with N elements`，
/// N=1 时单数 `element`；expecting 已含 `struct` 前缀，与 derive visitor 文
/// 本逐字节一致）。
pub(crate) fn invalid_length_message(index: usize, expecting: &str, len: usize) -> String {
    let plural = if len == 1 { "" } else { "s" };
    format!("invalid length {index}, expected {expecting} with {len} element{plural}")
}

/// serde_json 的 Display 在 line ≥ 1 时附带 ` at line L column C`；shape 候选
/// 只保留裸消息（位置由 span 承载，payload 为 `Error::custom` 的 0:0 形态）。
pub(crate) fn strip_position_suffix(message: String) -> String {
    match message.rfind(" at line ") {
        Some(index) => message[..index].to_owned(),
        None => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Probe {
        A,
        B,
    }

    const PROBE_VARIANTS: &[&str] = &["a", "b"];
    const PROBE_TABLE: &[(&str, Probe)] = &[("a", Probe::A), ("b", Probe::B)];

    fn raw_token(input: &[u8]) -> &RawValue {
        serde_json::from_slice(input).expect("测试输入必是合法 JSON token")
    }

    #[test]
    fn canonical_path_joins_fields_and_indices_without_leading_dot() {
        let mut ctx = Ctx::new(b"", NoLocations);
        assert_eq!(ctx.canonical_path(), "$");
        ctx.push_field("laneGraph");
        ctx.push_field("edges");
        let mark = ctx.push_index(0);
        ctx.push_field("speedLimit");
        // 与 serde_path_to_error 输出逐一对齐（深度 1 无前导点）。
        assert_eq!(ctx.canonical_path(), "laneGraph.edges[0].speedLimit");
        ctx.truncate(mark);
        assert_eq!(ctx.canonical_path(), "laneGraph.edges");
    }

    #[test]
    fn serde_message_builders_match_derive_byte_for_byte() {
        assert_eq!(
            missing_field_message("formatVersion"),
            "missing field `formatVersion`"
        );
        assert_eq!(
            duplicate_field_message("formatVersion"),
            "duplicate field `formatVersion`"
        );
        // unknown_field 手工复刻与 serde_core `unknown_field`/`OneOf` 逐字节一致。
        for names in [&["a"][..], &["a", "b"][..], &["a", "b", "c"][..]] {
            let specs: Vec<FieldSpec> = names.iter().map(|name| req(name)).collect();
            assert_eq!(
                unknown_field_message("bogus", &specs),
                <serde_json::Error as de::Error>::unknown_field("bogus", names).to_string()
            );
        }
        assert_eq!(
            invalid_length_message(1, "struct WireUnits", 2),
            "invalid length 1, expected struct WireUnits with 2 elements"
        );
    }

    #[test]
    fn decode_enum_accepts_known_variant_and_reports_unknown() {
        let input = br#""b""#;
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let value = decode_enum(&mut ctx, token, range, "Probe", PROBE_VARIANTS, PROBE_TABLE)
            .expect("已知 variant 必须解码成功");
        assert_eq!(value, Probe::B);

        // 计数器为线程局部且只在根驱动时清空：同测试内多次 replay 不同输入需
        // 显式 reset（两输入起点都是 0，否则会误触唯一性硬断言）。
        #[cfg(debug_assertions)]
        crate::counters::reset();
        let input = br#""bogus""#;
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let failure = decode_enum(&mut ctx, token, range, "Probe", PROBE_VARIANTS, PROBE_TABLE)
            .expect_err("未知 variant");
        let ReplayFailure::Shape(candidate) = failure else {
            panic!("unknown variant 是 Data 类 shape 候选：{failure:?}");
        };
        assert_eq!(
            candidate.message,
            "unknown variant `bogus`, expected `a` or `b`"
        );
        assert_eq!(candidate.path, "$");
        assert_eq!(candidate.anchor, ByteRange::new(0, 7));
    }

    /// R3-4：非字符串/非 `{` 的枚举标量保留原生 Syntax category（真实
    /// serde `expected value` 延迟 syntax，token 局部位置重建为全局
    /// override），不再归一为 Shape。
    #[test]
    fn decode_enum_preserves_syntax_category_for_non_string_scalar() {
        let input = b"1";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let failure = decode_enum(&mut ctx, token, range, "Probe", PROBE_VARIANTS, PROBE_TABLE)
            .expect_err("标量枚举");
        let ReplayFailure::Syntax(deferred) = failure else {
            panic!("标量枚举必须保留 Syntax category：{failure:?}");
        };
        assert_eq!(deferred.source.classify(), Category::Syntax);
        assert_eq!(
            deferred.source.to_string(),
            "expected value at line 1 column 1"
        );
        assert_eq!(deferred.path, "$");
        assert_eq!(deferred.position, (1, 1));
        assert_eq!(deferred.token_start, 0);
    }

    /// R3-7：四轴 point 恢复 `[f64; 3]` derive 行为——读恰好 3 轴后第 4 元素
    /// 由 serde_json 报真实 `trailing characters`（Syntax 延迟 syntax）；两
    /// 轴仍报 serde 定长数组的 `invalid length 2`（Data → shape 候选）。
    #[test]
    fn decode_point_preserves_fixed_array_category_split() {
        let input = b"[1,2,3,4]";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let failure = decode_point(&mut ctx, token, range).expect_err("四轴 point");
        let ReplayFailure::Syntax(deferred) = failure else {
            panic!("四轴 point 必须保留 Syntax category：{failure:?}");
        };
        assert_eq!(deferred.source.classify(), Category::Syntax);
        assert!(
            deferred
                .source
                .to_string()
                .starts_with("trailing characters"),
            "四轴 point 消息：{}",
            deferred.source
        );
        assert_eq!(deferred.path, "$");
        assert_eq!(deferred.token_start, 0);

        // 两轴同理（reset 原因见 decode_enum 测试）。
        #[cfg(debug_assertions)]
        crate::counters::reset();
        let input = b"[1,2]";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let failure = decode_point(&mut ctx, token, range).expect_err("两轴 point");
        let ReplayFailure::Shape(candidate) = failure else {
            panic!("两轴 point 是 Data 类 shape 候选：{failure:?}");
        };
        assert_eq!(
            candidate.message,
            "invalid length 2, expected an array of length 3"
        );
        assert_eq!(candidate.anchor, ByteRange::new(0, 5));
    }

    /// R3-1：超出 u64 的纯整数经 serde_json 回退为 f64 解码（invalid type
    /// floating point，Data → shape 候选）；超出 f64 的 numeric literal 保
    /// 留原生 Syntax category（`number out of range` → 延迟 syntax）。
    #[test]
    fn decode_scalar_preserves_syntax_category_for_out_of_range_numbers() {
        let input = b"99999999999999999999999999";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let failure =
            decode_scalar::<u64, NoLocations>(&mut ctx, token, range).expect_err("超 u64 整数");
        let ReplayFailure::Shape(candidate) = failure else {
            panic!("超 u64 整数是 Data 类 shape 候选：{failure:?}");
        };
        assert_eq!(
            candidate.message,
            "invalid type: floating point `1e+26`, expected u64"
        );
        assert_eq!(candidate.anchor, ByteRange::new(0, 26));

        #[cfg(debug_assertions)]
        crate::counters::reset();
        let input = b"1e999";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let failure =
            decode_scalar::<u64, NoLocations>(&mut ctx, token, range).expect_err("超 f64 字面量");
        let ReplayFailure::Syntax(deferred) = failure else {
            panic!("超 f64 字面量必须保留 Syntax category：{failure:?}");
        };
        assert_eq!(deferred.source.classify(), Category::Syntax);
        assert_eq!(
            deferred.source.to_string(),
            "number out of range at line 1 column 5"
        );
        assert_eq!(deferred.path, "$");
        assert_eq!(deferred.position, (1, 5));
        assert_eq!(deferred.token_start, 0);
    }

    #[test]
    fn strip_position_suffix_keeps_bare_message() {
        assert_eq!(
            strip_position_suffix(
                "invalid type: integer `1`, expected a string at line 3 column 5".to_owned()
            ),
            "invalid type: integer `1`, expected a string"
        );
        // line 0（Error::custom 形态）无后缀，原样保留。
        assert_eq!(
            strip_position_suffix("missing field `formatVersion`".to_owned()),
            "missing field `formatVersion`"
        );
    }
}
