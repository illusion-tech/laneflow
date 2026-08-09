//! 共享 visitor 游走层：record token 捕获、位置策略 hook、标量 replay 解码与
//! shape 候选机制。
//!
//! 机制（docs/design/current-package-import.md §7）：根文档由
//! `serde_json::Deserializer` 流式驱动；每个 field/element 的 value 先捕获为
//! `&RawValue` token（捕获即完成该子树的完整语法校验，且 token 借自原始输
//! 入，指针算术即得全局零基半开 byte 区间）；随后对 token 至多 replay 解码
//! 一次。token 必是合法 JSON，replay 内的失败永远归一为 shape 候选。

use std::fmt::{self, Write as _};

use serde::Deserialize;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde_json::value::RawValue;

use super::anchor::ByteRange;

/// visit_map/visit_seq 的中止信号：shape 候选经 `failure` 外传，该消息永不浮
/// 出水面（凡 sentinel 出现处 `failure` 必为 `Some`）。
const SENTINEL: &str = "laneflow shape candidate sentinel";

/// 延迟 shape 候选：`path` 为规范 `$` 形式；`message` 为裸 serde 消息（无位
/// 置后缀）；`anchor` 为全局零基半开 byte 区间。
#[derive(Debug)]
pub(crate) struct ShapeCandidate {
    pub(crate) path: String,
    pub(crate) message: String,
    pub(crate) anchor: ByteRange,
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

/// object 的 DeserializeSeed/Visitor 合体：捕获 value token、打位置 hook、调
/// handler（参数 `(ctx, key, value token, value 区间, push 前 path 标记)`）；
/// handler 失败时存候选并以 sentinel 中止。
pub(crate) struct ObjectSeed<'a, 'de, L, F> {
    ctx: &'a mut Ctx<'de, L>,
    failure: &'a mut Option<ShapeCandidate>,
    expecting: &'static str,
    handler: F,
}

impl<'de, L, F> DeserializeSeed<'de> for ObjectSeed<'_, 'de, L, F>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ShapeCandidate>,
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
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ShapeCandidate>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.expecting)
    }

    fn visit_map<A>(mut self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while let Some(key) = map.next_key::<&str>()? {
            let mark = self.ctx.push_field(key);
            let value = map.next_value::<&RawValue>()?;
            let range = self.ctx.token_range(value);
            self.ctx.policy_value(range);
            match (self.handler)(self.ctx, key, value, range, mark) {
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
    failure: &'a mut Option<ShapeCandidate>,
    expecting: &'static str,
    handler: F,
}

impl<'de, L, F> DeserializeSeed<'de> for ArraySeed<'_, 'de, L, F>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, usize, &'de RawValue, ByteRange) -> Result<(), ShapeCandidate>,
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
    F: FnMut(&mut Ctx<'de, L>, usize, &'de RawValue, ByteRange) -> Result<(), ShapeCandidate>,
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
/// 包装的 `Err`）。
pub(crate) fn drive_object<'de, L, F, D>(
    ctx: &mut Ctx<'de, L>,
    failure: &mut Option<ShapeCandidate>,
    expecting: &'static str,
    deserializer: D,
    handler: F,
) -> Result<(), serde_json::Error>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ShapeCandidate>,
    D: serde::Deserializer<'de, Error = serde_json::Error>,
{
    ObjectSeed {
        ctx,
        failure,
        expecting,
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

/// replay 解码 record token：handler 候选直接外传；真实 serde 错误（token 内
/// 只可能是 shape 语义失败）归一为以容器 token 为锚的候选。
pub(crate) fn decode_record<'de, L, F>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    expecting: &'static str,
    handler: F,
) -> Result<(), ShapeCandidate>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ShapeCandidate>,
{
    ctx.policy_record(range);
    count_replay(range);
    let mut failure = None;
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    let result = drive_object(ctx, &mut failure, expecting, &mut deserializer, handler);
    if let Some(candidate) = failure {
        return Err(candidate);
    }
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(ctx.candidate(strip_position_suffix(error.to_string()), range)),
    }
}

/// untagged 分派专用：完整扫描 record（handler 只记录、不报错）；`Ok(true)`
/// 表示结构干净，`Ok(false)` 表示出现真实 serde 错误（由调用方归一化为
/// mismatch 候选）。handler 主动产出的候选仍按 `Err` 传播。
pub(crate) fn scan_record<'de, L, F>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    expecting: &'static str,
    handler: F,
) -> Result<bool, ShapeCandidate>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, &str, &'de RawValue, ByteRange, usize) -> Result<(), ShapeCandidate>,
{
    ctx.policy_record(range);
    count_replay(range);
    let mut failure = None;
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    let result = drive_object(ctx, &mut failure, expecting, &mut deserializer, handler);
    if let Some(candidate) = failure {
        return Err(candidate);
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
) -> Result<(), ShapeCandidate>
where
    L: LocationPolicy,
    F: FnMut(&mut Ctx<'de, L>, usize, &'de RawValue, ByteRange) -> Result<(), ShapeCandidate>,
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
    if let Some(candidate) = failure {
        return Err(candidate);
    }
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(ctx.candidate(strip_position_suffix(error.to_string()), range)),
    }
}

/// replay 解码标量/透明值 token（String、u32、u64、f64、Vec<Value> 等）；失败
/// 消息剥离位置后缀后以当前 path 与 value token 为锚归一为候选。
pub(crate) fn decode_scalar<'de, T, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<T, ShapeCandidate>
where
    T: Deserialize<'de>,
    L: LocationPolicy,
{
    count_replay(range);
    let mut deserializer = serde_json::Deserializer::from_slice(token.get().as_bytes());
    T::deserialize(&mut deserializer)
        .map_err(|error| ctx.candidate(strip_position_suffix(error.to_string()), range))
}

/// 字符串枚举 token：经 `deserialize_enum` 复刻 derive 的 visitor 流转（字符串
/// 经 `visit_str` 查表报 `unknown variant`；非字符串沿用 serde_json 的
/// `expected value`/map-form 行为），expecting 文本 `enum {name}` 与 derive
/// 逐字节一致。
pub(crate) fn decode_enum<'de, T, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
    name: &'static str,
    variants: &'static [&'static str],
    table: &'static [(&'static str, T)],
) -> Result<T, ShapeCandidate>
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
    .map_err(|error| ctx.candidate(strip_position_suffix(error.to_string()), range))
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
) -> Result<(), ShapeCandidate>
where
    L: LocationPolicy,
{
    if token.get().trim() == "null" {
        return Err(ctx.candidate(NON_NULL_MESSAGE.to_owned(), range));
    }
    Ok(())
}

/// 拒绝显式 null 的 Option<String> 字段（extendsId/laneGroupId/source）。
pub(crate) fn decode_non_null_string<'de, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<String, ShapeCandidate>
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
) -> Result<String, ShapeCandidate>
where
    L: LocationPolicy,
{
    let lexeme = token.get().trim();
    if lexeme == "null" {
        return Err(ctx.candidate(NON_NULL_MESSAGE.to_owned(), range));
    }
    if is_json_number_lexeme(lexeme) {
        Ok(lexeme.to_owned())
    } else {
        Err(ctx.candidate(
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
) -> Result<bool, ShapeCandidate>
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

/// 根 `extensions`：内容不透明（任何合法 object 都接受）；非 object 借用
/// serde 的 invalid type 消息（`expected a map`）。
pub(crate) fn check_extensions_object<'de, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<(), ShapeCandidate>
where
    L: LocationPolicy,
{
    if token.get().trim_start().starts_with('{') {
        return Ok(());
    }
    decode_scalar::<serde_json::Map<String, serde_json::Value>, L>(ctx, token, range).map(|_| ())
}

/// centerline point `[f64; 3]`：逐轴解码（轴级 path/锚）；元素数不为 3 时以
/// serde 定长数组的 invalid length 消息归一（锚=point token）。
pub(crate) fn decode_point<'de, L>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<[f64; 3], ShapeCandidate>
where
    L: LocationPolicy,
{
    ctx.policy_point(range);
    let mut axes = [0.0_f64; 3];
    let mut count = 0_usize;
    decode_array(
        ctx,
        token,
        range,
        "an array of length 3",
        |ctx, index, element, element_range| {
            if index < 3 {
                axes[index] = decode_scalar::<f64, L>(ctx, element, element_range)?;
            }
            count += 1;
            Ok(())
        },
    )?;
    if count != 3 {
        return Err(ctx.candidate(
            format!("invalid length {count}, expected an array of length 3"),
            range,
        ));
    }
    Ok(axes)
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
) -> Result<(), ShapeCandidate>
where
    L: LocationPolicy,
    F: FnOnce(&mut Ctx<'de, L>, &'de RawValue, ByteRange) -> Result<T, ShapeCandidate>,
{
    if slot.is_some() {
        return Err(ctx.candidate_at(mark, duplicate_field_message(key), range));
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

/// `unknown field` 消息（与 serde derive 逐字节一致，含 expected 列表）。
pub(crate) fn unknown_field_message(field: &str, expected: &'static [&'static str]) -> String {
    <serde_json::Error as de::Error>::unknown_field(field, expected).to_string()
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
        assert_eq!(
            unknown_field_message("bogus", &["a"]),
            "unknown field `bogus`, expected `a`"
        );
        assert_eq!(
            unknown_field_message("bogus", &["a", "b"]),
            "unknown field `bogus`, expected `a` or `b`"
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
        let candidate = decode_enum(&mut ctx, token, range, "Probe", PROBE_VARIANTS, PROBE_TABLE)
            .expect_err("未知 variant");
        assert_eq!(
            candidate.message,
            "unknown variant `bogus`, expected `a` or `b`"
        );
        assert_eq!(candidate.path, "$");
        assert_eq!(candidate.anchor, ByteRange::new(0, 7));
    }

    /// 归一差异①（报告记录）：非字符串/非 `{` 的枚举标量在 serde_json 内部是
    /// Syntax 类（`expected value`），新架构把 token 内一切失败归一为 Shape
    /// 候选（旧实现报 JsonSyntax）。
    #[test]
    fn decode_enum_normalizes_non_string_scalar_to_shape() {
        let input = b"1";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let candidate = decode_enum(&mut ctx, token, range, "Probe", PROBE_VARIANTS, PROBE_TABLE)
            .expect_err("标量枚举");
        assert_eq!(candidate.message, "expected value");
        assert_eq!(candidate.anchor, ByteRange::new(0, 1));
    }

    /// 归一差异②（报告记录）：定长数组第 4 元素在旧实现经 serde 定长数组
    /// visitor 报 `trailing characters`（Syntax）；新实现逐轴计数后以
    /// invalid length 归一为 Shape。
    #[test]
    fn decode_point_rejects_four_axes_with_invalid_length() {
        let input = b"[1,2,3,4]";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let candidate = decode_point(&mut ctx, token, range).expect_err("四轴 point");
        assert_eq!(
            candidate.message,
            "invalid length 4, expected an array of length 3"
        );
        assert_eq!(candidate.anchor, ByteRange::new(0, 9));
        // 两轴同理（reset 原因见 decode_enum 测试）。
        #[cfg(debug_assertions)]
        crate::counters::reset();
        let input = b"[1,2]";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let candidate = decode_point(&mut ctx, token, range).expect_err("两轴 point");
        assert_eq!(
            candidate.message,
            "invalid length 2, expected an array of length 3"
        );
    }

    /// 归一差异③（报告记录）：超出 u64 的纯整数经 serde_json 回退为 f64 解
    /// 码（invalid type floating point），超出 f64 的 numeric literal 是其内部
    /// Syntax 类（`number out of range`）；两种形态新架构都归一为 Shape。
    #[test]
    fn decode_scalar_normalizes_out_of_range_numbers_to_shape() {
        let input = b"99999999999999999999999999";
        let mut ctx = Ctx::new(input, NoLocations);
        let token = raw_token(input);
        let range = ctx.token_range(token);
        let candidate =
            decode_scalar::<u64, NoLocations>(&mut ctx, token, range).expect_err("超 u64 整数");
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
        let candidate =
            decode_scalar::<u64, NoLocations>(&mut ctx, token, range).expect_err("超 f64 字面量");
        assert_eq!(candidate.message, "number out of range");
        assert_eq!(candidate.anchor, ByteRange::new(0, 5));
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
