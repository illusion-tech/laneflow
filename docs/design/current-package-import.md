# 当前 Traffic/Spatial 包迁移导入前端

**文档状态**: Review（#297 G1 候选；未授权 G2）<br>
**最后更新**: 2026-08-06<br>
**适用范围**: Traffic v0.10、SpatialPackage v0.1、ScenarioManifest v0.1、
`laneflow-current-source`、`laneflow-compiler` 的 `current-v0_10-import` 迁移特性、
`laneflow-current-import`、current → canonical LIR 迁移与资产审计<br>
**实现状态**: 未实现；当前生产加载仍由 `laneflow-data` 完成，本文件只冻结 #297 的
G1 实现输入

**关联决策与设计**:

- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `compiler-foundation.md`
- `network-compiler.md`
- `data-format.md`
- `data-loading.md`
- `spatial-geometry.md`
- `../reference/glossary.md`

## 1. 目标与非目标

#297 交付一次性的当前态来源迁移前端。它把一份 ScenarioManifest v0.1 与其精确绑定的
Traffic v0.10、SpatialPackage v0.1 原始字节作为一个逻辑来源模块导入现有编译器，
保留三个独立来源文档、真实来源位置和 current 外部标识，再进入共同 Typed AST → HIR
→ MIR → canonical LIR 管线。

本切片不把 current JSON 重新定义为长期编制来源，不建立第二条运行时加载路径，不让
compiler 依赖 `InitialTrafficData`、`LaneGraph`、`SpatialRegistry` 或其他 current
Core/Spatial 对象图，也不在 importer 中复制任何 HIR/MIR/LIR 语义规则。阶段 8 切换后，
生产 Runtime 不再解析 current JSON；是否继续保留离线迁移工具由后继治理决定。

无需 Spatial 的 Traffic-only current Core 入口继续存在。它只表示 current Traffic
加载，不得冒充已经完成 ScenarioManifest/Traffic/Spatial 三文档绑定的 compiler 导入。

## 2. 当前实现事实与待替换边界

截至 `main@0b95362167792d7bbde5f9a359103bde47066ed2`：

- `laneflow-data` 的 `from_scenario_json_slice` 借用 Manifest 和 `NamedArtifact[]`，验证
  Manifest/Traffic/Spatial 版本、角色媒体类型、声明长度、SHA-256 和引用配对，再分别
  构造 current Core/Spatial 输入；
- ScenarioManifest 只声明 Traffic/Spatial 两个角色，但调用方制品集合允许任意数量的
  唯一额外制品；额外制品只参与非空/唯一检查，不被哈希或解析；
- current 生产路径没有制品数、单个引用、引用总字节、文档字节、记录数、位置数或
  加载存续内存上限；#297 不得把严格 compiler 上限回写为该路径的新拒绝条件；
- compiler 已有 `LF-COMP-P100-INITIAL-v2`、三文档共同准入能力、私有
  `AdmittedOfficialModule`、原子 `prepare_admission`/`commit_admission` 和三文档源映射
  冻结；#297 只补 current 专用来源验证与降阶；
- 仓库完整 paired fixtures 的最大值来自 signalized corridor：Manifest 522 字节、
  Traffic 85,339 字节、Spatial 151,268 字节，合计 237,129 字节；三文档合计 8,036 个
  JSON 值、6,352 个数组成员，Traffic/Spatial 语义字符串最大 53 字节。该事实只用于
  校准迁移配置档，不改变格式 schema。

## 3. 包依赖图、特性与退役

箭头表示左侧正常依赖右侧：

```text
laneflow-compiler ----------------> laneflow-static-contract
laneflow-compiler --[current-v0_10-import]--> laneflow-current-source

laneflow-current-import --------------------> laneflow-compiler

laneflow-data ------------------------------> laneflow-current-source
laneflow-data ------------------------------> laneflow-core
laneflow-data ------------------------------> laneflow-spatial
```

三个 current 相关包均设置 `publish = false`。`laneflow-current-source` 只依赖 Rust
标准库、Serde/serde_json、SHA-256 与错误支持，不依赖 compiler、Core 或 Spatial。
`laneflow-compiler` 默认特性集合不含 current；只有默认关闭的
`current-v0_10-import` 特性加入对 source 包的依赖和公开的借用输入入口。
`laneflow-current-import` 的正常依赖只能是启用该特性的 compiler；Cargo 图和
compile-fail 测试必须证明 importer 不能直接命名 `laneflow-current-source`、受检包、
严格配置或 compiler 私有模块构建器。

`laneflow-data` 在 #297 中改为消费 production-compatible source 能力，再继续拥有
current Core/Spatial 规范化。它不得保留第二份 Traffic/Spatial/Manifest wire DTO、版本
判断、摘要或配对实现。Traffic-only Data 入口消费 source 包的独立 Traffic-only
production-compatible 能力，不强制虚构 Manifest 或 Spatial。

退役分两步：#294 生产切换时删除 `laneflow-data` 到 current Core/Spatial 的运行时 JSON
路径；#297 的资产审计、发布迁移说明和约定保留期完成后，另由治理变更删除
`current-v0_10-import`、`laneflow-current-import` 与 `laneflow-current-source`。不得仅因
Runtime 已切换便先删除唯一离线迁移入口。

## 4. Compiler 公共借用输入

下列类型只在 `current-v0_10-import` 特性下公开。字段保持私有；全部构造器为 `const`、
不失败、不分配、不复制、不解析、不哈希，也不把输入声明为已验证：

```rust
#[derive(Clone, Copy, Debug)]
pub struct CurrentSourceArtifact<'a> { /* private */ }

impl<'a> CurrentSourceArtifact<'a> {
    pub const fn new(
        artifact_ref: &'a str,
        bytes: &'a [u8],
        display_source: Option<&'a str>,
    ) -> Self;

    pub const fn artifact_ref(self) -> &'a str;
    pub const fn bytes(self) -> &'a [u8];
    pub const fn display_source(self) -> Option<&'a str>;
}

#[derive(Clone, Copy, Debug)]
pub struct CurrentSourceInput<'a> { /* private */ }

impl<'a> CurrentSourceInput<'a> {
    pub const fn new(
        manifest_bytes: &'a [u8],
        manifest_display_source: Option<&'a str>,
        artifacts: &'a [CurrentSourceArtifact<'a>],
    ) -> Self;
}

impl CompilationUnitBuilder {
    pub fn add_current_source(
        &mut self,
        source: CurrentSourceInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle>;
}
```

一个生命周期参数同时约束切片及其元素，防止 importer 用短生命周期元数据拼出长生命
周期输入。`display_source` 是未认证的显示/审计字符串，可以是仓库相对路径、资产键或
宿主标签；它不参与摘要、标识或发布真实性。

`add_current_source` 是唯一 compiler 提交入口。compiler 不公开接收
`ValidatedCurrentImportBundle`、`CurrentSourceLimits`、当前 DTO、位置表、来源描述符或
私有 `CurrentImportModule` 的二段式 API。

## 5. Source 包能力 API

`laneflow-current-source` 暴露三条具体、不可互换的能力路径：

```rust
#[derive(Clone, Copy, Debug)]
pub struct CurrentDocumentInput<'a> { /* private */ }

impl<'a> CurrentDocumentInput<'a> {
    pub const fn new(bytes: &'a [u8], display_source: Option<&'a str>) -> Self;
}

#[derive(Clone, Copy, Debug)]
pub struct CurrentArtifactInput<'a> { /* private */ }

impl<'a> CurrentArtifactInput<'a> {
    pub const fn new(
        artifact_ref: &'a str,
        bytes: &'a [u8],
        display_source: Option<&'a str>,
    ) -> Self;
}

pub fn validate_traffic_compatible(
    traffic_bytes: &[u8],
) -> Result<ValidatedCurrentTrafficPackage, CurrentSourceError>;

pub fn validate_scenario_compatible(
    manifest_bytes: &[u8],
    artifacts: &[CurrentArtifactInput<'_>],
) -> Result<ValidatedCurrentSourceBundle, CurrentSourceError>;

#[doc(hidden)]
pub fn validate_scenario_strict(
    manifest: CurrentDocumentInput<'_>,
    artifacts: &[CurrentArtifactInput<'_>],
    limits: &CurrentSourceLimits,
) -> Result<ValidatedCurrentImportBundle, CurrentSourceError>;
```

Source 输入类型与 compiler 输入类型有意不同：前者是 source 包的跨包实现入口，后者
保证 importer 的正常依赖闭包只需 compiler。compiler 在 `add_current_source` 栈内把两个
借用视图逐项转换，不复制底层字符串或字节。由于 Rust 不能安全地把两个不同 newtype 的
切片重解释为同一布局，compiler 先 allocation-free 检查制品数和 ref 长度，再为最多 16
个 source view 请求一个受事务 live budget 约束的短 `Vec`；source 包仍重新执行权威
preflight。该转换只复制三个借用指针/长度元数据，不复制 payload。

前两条只供 current production façade；第三条因 Rust 不存在 friend crate 而必须跨包
可见，但官方路径只由 compiler 调用。任意外部程序自行依赖 source 包并调用严格入口
属于 caller-owned 工作；它不能把结果提交给 compiler，也不在 compiler 资源保证内。

`ValidatedCurrentTrafficPackage`、`ValidatedCurrentSourceBundle` 和
`ValidatedCurrentImportBundle` 均无公开字段、`Default`、Serde 实现或裸构造器。
production scenario 能力原子拥有三份已验证 wire 内容、精确文档摘要和 Manifest
配对；它不包含逐文档 compiler origin 或位置表。strict 能力不可分地拥有相同来源包、
三个逐文档 origin、受限位置数据和精确资源用量。production 能力不能升级为 strict
能力，strict 能力也不能降级后再提交。

跨包消费 current DTO 所需的只读/按值视图可以是 `pub`，但 source 包保持未发布，且
这些视图只逐项对应冻结的 current wire 字段；它们不是第三方 authoring API。Data 与
compiler 必须消费同一 DTO，不得各自再反序列化原始 JSON。

## 6. 文档身份、来源语言与来源记录

三文档共同形成一个无导入边的逻辑模块：

| 项目                         | 精确值                                                      |
| ---------------------------- | ----------------------------------------------------------- |
| `authoringNamespaceId`       | `current/v0.10`                                             |
| `SourceLanguage` 新值        | `CurrentTrafficSpatialV0_10 = 2`                            |
| `SourceLanguage::as_str()`   | `current-traffic-spatial-v0.10`                             |
| `frontendVersion`            | `1`                                                         |
| Manifest `sourceDocumentKey` | `current/v0.10/manifest`                                    |
| Traffic `sourceDocumentKey`  | `current/v0.10/traffic`                                     |
| Spatial `sourceDocumentKey`  | `current/v0.10/spatial`                                     |
| imports                      | 空集合                                                      |
| `generatorBuildId`           | `laneflow-current-import/v1`                                |
| `frontendOptionsDigest`      | SHA-256(`LFCURRENT-IMPORT-OPTIONS` + little-endian `1_u32`) |
| `parametersAndInputsDigest`  | Manifest 原始字节 SHA-256                                   |
| `randomSeed`                 | `None`                                                      |
| `provenance`                 | `current-package-migration`                                 |

稳定文档键不含宿主路径、`artifactRef`、输入数组下标或内容摘要。固定 namespace 意味着
一个编译单元至多接收一个 current 场景；第二次加入由现有重复 namespace 诊断原子拒绝。

三份 `SourceDocumentDescriptor` 的摘要都对各自原始字节计算一次 SHA-256，长度是原始
字节长度；模块文档集摘要继续使用 #315 已实现的 v1 规范聚合。逐文档 origin 精确为：

| 角色     | `artifactRef`                     | `displaySource`                          |
| -------- | --------------------------------- | ---------------------------------------- |
| Manifest | `None`                            | `CurrentSourceInput::new` 的 manifest 值 |
| Traffic  | Manifest 已绑定的精确 Traffic ref | 被选中输入制品的值                       |
| Spatial  | Manifest 已绑定的精确 Spatial ref | 被选中输入制品的值                       |

compiler 的查询面也只在迁移特性下增加，不把 current 专用角色永久放入默认构建：

```rust
#[cfg(feature = "current-v0_10-import")]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CurrentSourceDocumentRole {
    Manifest,
    Traffic,
    Spatial,
}

impl SourceDocumentOrigin {
    #[cfg(feature = "current-v0_10-import")]
    pub const fn current_document_role(&self) -> Option<CurrentSourceDocumentRole>;

    #[cfg(feature = "current-v0_10-import")]
    pub fn artifact_ref(&self) -> Option<&str>;
}
```

Manifest 的 role 为 `Some(Manifest)`、artifact ref 为 `None`；Traffic/Spatial 两项均为
相应 role 和 `Some` 精确 ref；Synthetic 两项均为 `None`。origin 字段保持私有，公开
枚举值不能用于伪造文档描述符。feature 关闭时枚举、两个 accessor 及 origin 内的
current-only 存储都不存在；feature 退役时一并删除。角色按固定 slot 计 live/retained，
两个选中 ref 和三个 display source 按实际 String item/bytes 计共同资源。

角色、引用和显示来源不进入内容摘要、文档集摘要、StableId 或 LIR 语义，但要计入字符串和
compiler-controlled live bytes。原始输入只借用到 `add_current_source` 返回；成功结果只
保留摘要、长度、角色、已复制的 origin 字符串及源映射位置，不保留来源全文。

## 7. 单一解析与配对实现

Source 包使用一套私有、按位置策略泛型化的 Serde wire DTO：production 实例化
`NoLocations`，strict 实例化 `CaptureLocations`。字段集合、unknown-field 拒绝、可选字段
的 explicit-null 规则、版本、数字词法、摘要和配对函数只有一份；位置策略使用静态分派，
记录循环中没有 trait object 或运行时 policy 分支。

为在一次文档级遍历中保留现有的 syntax → version → shape 优先级，顶层与嵌套容器使用
手写 `DeserializeSeed`/visitor：JSON 词法或结构无法安全继续时立即返回原始 syntax 错误；
可安全跳过的类型、unknown field 和缺字段问题只保存首个有类型 shape 候选并继续消费当前
JSON 值，以便取得 `formatVersion`。完整 JSON 有效后先裁决 unsupported version，再返回
shape 候选。production 只保存首个候选；strict 可在诊断上限内保存多个候选。visitor
不得构造 `serde_json::Value` 或 Serde `Content` 树；允许对单个借用 `RawValue` token
就地解码，但不得重放整个文档或完整容器子树。

strict 的 `CaptureLocations` 对需要定位的标量使用借用 `RawValue` 新类型：先以原始 token
长度执行字符串/数值增长前检查，再在同一字段 visitor 内完成词法解码，并把 token 在原始
输入切片中的 `u32` 起止 byte offset 记入有类型位置槽。它不对文档执行第二次 DTO 解析，
也不扫描序列来事后重建字段路径。每份文档唯一一次 SHA-256 扫描同时收集换行 byte offset；
位置冻结时通过该有界换行索引把 byte offset 转为现有 `SourcePosition { line, column }`。
production 的 `NoLocations` 不建立换行索引、不携带位置字段，也不构造
`CurrentSourceLocationTable`。

`extensions` 与尚不可用的 `timeWindows` 子树仍执行完整 JSON 语法与 strict 资源计数；
它们不物化 compiler 位置条目。`timeWindows` 在字段根位置进入既有 capability-unavailable
诊断，`extensions` 继续被 current 语义忽略。production 必须以现有兼容测试证明 accepted
set 没有收窄；strict 可以按本文件配置档更早返回资源诊断。

Manifest 仍只允许 Traffic/Spatial 两个 descriptor。production 使用当前等价的
`HashMap<&str, &[u8]>` 查找策略保持任意数量制品的 O(n) 配对；strict 先检查最多 16 个输入，
再用 allocation-free O(n²) 比较完成非空/唯一性和线性目标查找。两个容器通过封闭的静态
lookup policy 调用同一配对函数；额外唯一制品的 bytes 在两种策略中都不被哈希、解析或复制。

strict 成功顺序固定为：

1. compiler 在读、哈希、解析前验证 v2/后继多文档配置档，且模块余额至少 1、文档余额
   至少 3；
2. allocation-free 检查制品数、每个 ref、ref 总字节，以及 Manifest 和每个输入制品的
   单项 display source；
3. 检查 Manifest 实际字节，计算一次 Manifest SHA-256/换行索引并有界解析；
4. 按 Traffic descriptor 的非空 ref → media type → portable size → digest 词法、Spatial
   descriptor 的相同顺序、两者 ref 冲突的既有优先级完成 Manifest 语义验证；
5. 对调用方 ref 集合执行非空/全集合唯一检查并定位两个目标，再检查 Manifest 与被选中
   Traffic/Spatial 三份 display source 的总字节；未引用制品的 display source 不进入
   origin 或该总量；
6. 在 Traffic/Spatial 摘要或 DTO 分配前检查声明长度、实际长度、单文档和三文档组合
   source 资源；资源失败允许提前，但非资源 size mismatch 只记录到下一步裁决；
7. 按 Traffic actual size → Traffic SHA-256/digest → Spatial actual size → Spatial
   SHA-256/digest 的既有优先级验证；每份摘要扫描同时建立其 strict 换行索引；
8. 按 Manifest → Traffic → Spatial 顺序有界解析，同步累计 wire、语义、位置和 live
   资源；
9. 返回一个不可拆的 strict 能力；compiler 随即降阶并执行共同候选复核；
10. 全部检查成功后调用现有 `commit_admission`，否则释放候选且 builder 不变。

production 在 source 层保持现有可观察的非资源失败顺序：Manifest syntax/version/shape →
Traffic descriptor → Spatial descriptor → conflicting ref → provided refs → Traffic
size/digest → Spatial size/digest → Traffic wire → Spatial wire。只有 strict 可以把新增的
资源失败提前。source 原子成功后，Data 仍按 Traffic → Spatial 执行 current
Core/Spatial 规范化；兼容承诺冻结 accepted set 和单故障结构化错误，不冻结一个输入同时
含 source 错误与后续 domain 错误时两者的相对首错优先级。

## 8. 严格来源资源配置档

固定配置档标识为 `LF-CURRENT-SOURCE-P100-IMPORT-v1`。它没有 `Default`、无限模式或环境
变量覆盖；修改任一固定值必须提升 profile ID。以下是 source 专用硬上限：

| `CurrentSourceLimitDimension` / 私有字段                       |   精确上限 | 计数对象 / 推导                         |
| -------------------------------------------------------------- | ---------: | --------------------------------------- |
| `ArtifactCount` / `max_artifact_count`                         |         16 | 调用方具名制品；允许 14 个唯一额外项    |
| `ArtifactReferenceBytes` / `max_artifact_ref_bytes`            |        256 | 单个 UTF-8 ref 原始字节                 |
| `ArtifactReferenceTotalBytes` / `max_artifact_ref_total_bytes` |      4,096 | `16 * 256`                              |
| `DisplaySourceBytes` / `max_display_source_bytes`              |      1,024 | 单文档显示来源 UTF-8 字节               |
| `DisplaySourceTotalBytes` / `max_display_source_total_bytes`   |      3,072 | 三文档显示来源总字节                    |
| `ManifestSourceBytes` / `max_manifest_source_bytes`            |     65,536 | Manifest 原始字节                       |
| `TrafficSourceBytes` / `max_traffic_source_bytes`              |    524,288 | Traffic 原始字节                        |
| `SpatialSourceBytes` / `max_spatial_source_bytes`              |    524,288 | Spatial 原始字节                        |
| `SourceBytesPerModule` / `max_source_bytes_per_module`         |    542,741 | 与 `LF-COMP-P100-INITIAL-v2` 相同       |
| `JsonDepth` / `max_json_depth`                                 |         32 | 根值深度为 1                            |
| `JsonValueCount` / `max_json_value_count`                      |    262,144 | object/array/scalar 值总数              |
| `WireRecordCount` / `max_wire_record_count`                    |     58,387 | 与 typed AST 首轮记录上限相同           |
| `SequenceItemCount` / `max_sequence_item_count`                |    131,072 | 全部数组成员，包括忽略子树              |
| `JsonStringBytes` / `max_json_string_bytes`                    |      4,096 | 任一已解码 JSON 字符串                  |
| `JsonStringTotalBytes` / `max_json_string_total_bytes`         |    542,741 | 全部已解码 JSON 字符串总字节            |
| `LocationCount` / `max_location_count`                         |    262,144 | 本文闭合集合中的位置条目                |
| `CurrentSourceLiveBytes` / `max_current_source_live_bytes`     | 25,165,824 | DTO、容量、换行索引、位置与 lookup 峰值 |

这些数值不是格式兼容承诺，也不是从墙钟时延反推：

- 三文档总字节直接继承现有 compiler 的 542,741 字节局部/累计上限；Traffic/Spatial
  单文档 512 KiB 只作先行失败边界，最终仍受更小的三文档总量约束。仓库最大 paired
  场景为 237,129 字节，保留 2.29 倍原始字节余量；
- 16 个制品和 256 字节 ref 相对仓库 paired 场景的 2 个目标、最长 45 字节 ref 留出
  14 个额外项和 5.68 倍单项余量，同时让 strict 唯一性检查无需输入规模索引；
- 131,072 个序列项大于首轮 common profile 中 reference、relation、identity field、route
  occurrence 与 geometry point 上限之和 101,424；262,144 个 JSON 值和位置分别覆盖该
  序列包络，并高于 `4 * 58,387 = 233,548`；
- 24 MiB source live 上限按 58,387 个 128-byte record slot、131,072 个 16-byte sequence
  slot、262,144 个 16-byte location slot、542,741 个 owned string byte、最坏每个来源
  byte 一个 `u32` 换行 offset，再加 2 MiB lookup/control reserve 求和为 18,575,849
  字节；剩余 6,589,975 字节吸收小容器与对齐，但所有实际 capacity 仍逐次精确计费。

profile 提升必须携带 repository/published asset 失败清单、上述算式重算和 release 峰值
证据；不得只因某个外部输入超限便原地放宽 v1。

`WireRecordCount` 计 object 记录和有语义的 point 三元组，不重复计 object 的字段；
`SequenceItemCount` 计每次数组出现。`JsonValueCount`、深度、字符串和序列在 visitor 增长
前饱和累计。哈希表请求字节沿用共同接入的保守八桶/控制字模型；Vec、Box、String 和
换行/位置表按请求 capacity 计入 live bytes，不能用成功后的 len 冒充峰值。

strict 还逐项接收从 `CompileLimits` 当前余额派生的共同模块维度；在空 builder 上其固定
上界继承 `LF-COMP-P100-INITIAL-v2`：

| 共同维度                                                  |           空 builder 上限 |
| --------------------------------------------------------- | ------------------------: |
| `ModuleCount` / `SourceDocumentCount` / `ImportEdgeCount` | `1 / 3 / 0`（本模块需求） |
| `DeclarationCount` / `SymbolCount`                        |         `11,265 / 11,265` |
| `TypedAstRecordCount`                                     |                    58,387 |
| `ReferenceCount`                                          |                    37,920 |
| `RelationOccurrenceCount`                                 |                    10,032 |
| `IdentityFieldOccurrenceCount`                            |                    29,184 |
| `RouteOccurrenceCount`                                    |                     1,920 |
| `ManeuverGateCount` / `WaitingZoneCount`                  |           `2,304 / 1,536` |
| `GeometryPointCount`                                      |                    22,368 |
| `StringItemCount`                                         |                    36,894 |
| `SingleStringBytes`                                       |                        53 |
| `TotalStringBytes`                                        |                   991,537 |
| `DiagnosticCount`                                         |                        16 |

compiler 向 source 包传递的动态部分使用具名字段，避免位置参数漂移：

```rust
#[doc(hidden)]
pub struct CurrentCompilerBudget {
    pub max_source_bytes_per_module: u64,
    pub max_source_bytes_total: u64,
    pub max_declaration_count: u64,
    pub max_typed_ast_record_count: u64,
    pub max_reference_count: u64,
    pub max_relation_occurrence_count: u64,
    pub max_identity_field_occurrence_count: u64,
    pub max_route_occurrence_count: u64,
    pub max_maneuver_gate_count: u64,
    pub max_waiting_zone_count: u64,
    pub max_geometry_point_count: u64,
    pub max_symbol_count: u64,
    pub max_string_item_count: u64,
    pub max_single_string_bytes: u64,
    pub max_total_string_bytes: u64,
    pub max_diagnostic_count: u64,
    pub max_import_transaction_live_bytes: u64,
}

impl CurrentSourceLimits {
    #[doc(hidden)]
    pub fn from_compiler_budget(
        budget: CurrentCompilerBudget,
    ) -> Result<Self, CurrentSourceLimitsError>;
}
```

该构造器要求 diagnostic 和 import transaction live budget 非零，并验证
`max_single_string_bytes <= max_total_string_bytes`；其他零值表示对应 builder 余额已经
耗尽，必须保留为可在读取/分配前失败的有效上限。随后逐维与固定 source profile 取
较小值；它不接受 profile ID 字符串、source 硬上限或
`SourceDocumentCount`。compiler 必须先自行确认所选 `CompileLimits` 显式支持至少三份
剩余文档，才可构造 budget。其他 crate 即使直接依赖 source 包并构造该值，也不能把其
结果提交给 compiler。

有效值不是该表的过期快照。`add_current_source` 持有 `&mut self` 后，用饱和/受检减法从
当前 `AdmissionTotals` 和 builder live bytes 派生余额：累计维度取
`min(source profile hard limit, compile limit - committed usage)`；局部维度取两个固定
上限的较小值。三文档总字节同时受 Manifest/Traffic/Spatial 单项、source profile 总量、
`SourceBytesPerModule` 和剩余 `SourceBytesTotal` 约束。

`max_import_transaction_live_bytes` 精确等于调用点剩余的
`CompilerControlledLiveBytes`，不另设无限值。解析期同时受 24 MiB source 峰值和该余额
约束；降阶时使用同一事务账本，把移动的字符串/向量从 source charge 转为 destination
charge，并在任何新 capacity 请求前检查
`committed_builder + remaining_source + destination_candidate`。位置被移入 `SourceSpan`
后立即解除对应 source charge；共同 admission 前 strict bundle 和位置表必须已释放。

语义字符串的 53 字节上限是 compiler profile，不改变 Traffic schema 的 128 字节接受
域。production-compatible Data 路径继续接受 schema/current loader 已接受的值；strict
导入超过 53 字节时返回资源诊断，而不是截断、哈希替代、改写 external ID 或隐式提升
compiler profile。

## 9. 结构化资源诊断

Source 包资源失败统一使用：

```rust
CurrentSourceErrorPayload::LimitExceeded {
    profile_id: "LF-CURRENT-SOURCE-P100-IMPORT-v1",
    dimension: CurrentSourceLimitDimension,
    limit: u64,
    observed: u64,
    phase: CurrentSourceLimitPhase,
    document: Option<CurrentDocumentRole>,
    path: Option<Box<str>>,
    span: Option<CurrentSourceSpan>,
}
```

`CurrentSourceLimitPhase` 闭合为 `InputPreflight`、`ManifestDecode`、`ArtifactBinding`、
`TrafficDecode`、`SpatialDecode`、`LocationFreeze`、`CompilerLowering`。compiler 映射为
`LF-COMP-CURRENT-SOURCE-LIMIT-EXCEEDED`，保留 profile、dimension、limit、observed、
phase、document、path 和真实 span；不得改写成 JSON shape 或普通 compile limit。

非资源错误继续分型为 JSON syntax/shape、unsupported version、empty/conflicting/
duplicate/missing artifact ref、media type、portable size、size mismatch、digest syntax/
mismatch。Source 错误保留 JSON error category、message、规范 `$` path 和真实 span。
`laneflow-data` 必须映射为现有 `DataError`/`ScenarioError` variant 及其 document、path、
line、column 字段：立即失败的 syntax 携带原始 `serde_json::Error`，延迟 shape 候选使用
`serde::de::Error::custom` 构造同为 Data category 的 `serde_json::Error`。兼容测试冻结公共
variant 和上述顶层字段，不要求嵌套 source error 的 `Display` 逐字节相同。compiler 为这些
错误分别使用 current-source syntax/shape/version/artifact-binding 诊断；共同语义错误继续
使用现有 compiler 诊断码和 canonical ordering。

诊断上限只截取规范排序后的前 16 条。资源 preflight、摘要/配对和无法安全继续解析的
syntax 错误为单一立即失败；可安全收集的同阶段 shape/semantic 候选按
`document role → start position → code → typed payload` 排序。错误文本不参与排序或摘要。

## 10. 来源位置闭合集合

`CurrentSourceLocationTable` 只保存下列位置；每项均为文档角色、有类型记录键/字段键和
真实起止位置。identity/owner/relation 的 span 必须与其目标键一起移动，不能依赖 current
数组下标与 canonical ordinal 相同。

### 10.1 Manifest

- `formatVersion`；
- Traffic/Spatial 各自的 `artifactRef`、`mediaType`、`digest`、`size`；
- descriptor 记录位置以其 `artifactRef` 值区间为锚。

### 10.2 Traffic

- 根：`formatVersion`、`units.distance`、`units.time`；
- `laneGraph.edges[]`：记录/`id`、`length`、`speedLimit`、每个
  `connections[].toEdgeId`；
- `junctions[]`：记录/`id`；`movements[]`：记录/`id`、`junctionId`；
- `maneuverPaths[]`：记录/`id`、`movementId`、`entryEdgeId`、每个
  `internalEdgeIds[]`、`exitEdgeId`；
- `routes[]`：记录/`id`、每个 `edgeIds[]`；
- `vehicleProfiles[]`：记录/`id`、`length`、`model`、`desiredSpeed`、`minGap`、
  `timeHeadway`、`maxAcceleration`、`comfortableDeceleration`、
  `emergencyDeceleration`、`participantClassId`；
- `participantClasses[]`：记录/`id`、可选 `extendsId`；
- `facilityBands[]`：记录/`id`、`kindId`；
- `roadSections[]`：记录/`id`、`kindId`；每个 lane 记录、`edgeIds` 字段根、每个
  `edgeIds[]` 及可选 `laneGroupId`；成功构造的 AuthoringLane identity 使用首个 edge
  span，空链诊断使用 `edgeIds` 字段根；
- `laneGroups[]`：记录/`id`、`roadSectionId`；
- `roadCorridors[]`：记录/`id`、`referenceSectionId`；每个 `elements[]` 记录及其唯一
  `sectionId` 或 `bandId`；
- `accessRules[]`：记录/`id`、`target.kind`、`target.id`、`effect`、每个
  `participantClassIds[]`、`timeWindows` 字段根、`regulation.jurisdiction`、
  `regulation.version`、可选 `regulation.source`、可选 `priority`；
- `waitingZones[]`：记录/`id`、`maneuverPathId`、`entryGateId`、`releaseGateId`、
  `maxOccupancy`；
- `signals.stopLines[]`：记录/`id`、`edgeId`、`location`；
- `signals.maneuverGates[]`：记录/`id`、`maneuverPathId`、`transitionIndex`、
  `stopLineId`、`signalControl.kind`、可选 `signalControl.groupId`；
- `signals.groups[]`：记录/`id`；
- `signals.controllers[]`：记录/`id`、`kind`、`offsetMs`、每个 `groupIds[]`；每个
  `phases[]` 的记录/`id`、`durationMs`；每个 `states[]` 记录/`groupId`、`aspect`；
- `parking.areas[]`：记录/`id`；
- `parking.spaces[]`：记录/`id`、可选 `areaId`、entry/exit 的 `edgeId`/`progress`，
  geometry 的 `lateralOffset`、`headingOffsetRadians`、`length`、`width`。

`extensions` 子树不产生 compiler 位置条目，但仍受 strict JSON 计数。所有派生 owner、
反向成员与 canonical relation 使用上述显式引用出现位置；不为同一关系制造第二个来源。

### 10.3 Spatial

- `formatVersion`、`frameId`；
- 每个 `edges[]` 记录及 `trafficEdgeId`；
- 每个 `centerline.points[]` 三元组记录，以及 `[0]`、`[1]`、`[2]` 三个坐标。

几何点进入 canonical order 时，point span 和 lane-edge target 随同一 typed relation
permutation 移动；不得假设 Spatial input edge/point local index 等于 LIR ordinal。

## 11. Current → compiler 降阶

所有 current external ID 大小写敏感、原样使用，不 trim、不 case-fold、不 Unicode
normalize。除下表明确列出的迁移派生 key 外，禁止使用数组下标、HashMap 顺序、
几何、摘要或运行时 handle 补 key：

| Current 来源                              | Compiler 声明 / 关系      | 稳定键规则                                                                            |
| ----------------------------------------- | ------------------------- | ------------------------------------------------------------------------------------- |
| `laneGraph.edges[].id`                    | `LaneEdge`                | 全部 edge 原样作为 `laneEdgeKey`                                                      |
| `junctions[].id`                          | `Junction`                | 原样                                                                                  |
| `movements[].id`                          | `Movement`                | `movementKey` 原样；directed entry/exit approach key 分别为 `<id>/entry`、`<id>/exit` |
| `maneuverPaths[].id`                      | `ManeuverPath`            | 原样；parent/edge sequence 来自显式引用                                               |
| `routes[].id`                             | `StaticRoute`             | 原样                                                                                  |
| `vehicleProfiles[].id`                    | `VehicleProfile`          | 原样；`model` 仍由共同 capability/shape 规则裁决                                      |
| `participantClasses[].id`                 | `ParticipantClass`        | 原样                                                                                  |
| `facilityBands[].id`                      | `FacilityBand`            | 原样；唯一 corridor owner 由 elements 求解                                            |
| `roadSections[].id`                       | `RoadSection`             | 原样；唯一 corridor owner 由 elements 求解                                            |
| `roadSections[].lanes[]`                  | `AuthoringLane`           | `authoringLaneKey = edgeIds[0]`；空链先失败，重复首 edge/coverage 由共同语义拒绝      |
| `laneGroups[].id`                         | `LaneGroup`               | 原样；parent 来自 `roadSectionId`                                                     |
| `roadCorridors[].id`                      | `RoadCorridor`            | 原样                                                                                  |
| `accessRules[].id`                        | `AccessRule`              | 原样                                                                                  |
| `waitingZones[].id`                       | `WaitingZone`             | 原样；parent 为 `maneuverPathId`                                                      |
| StopLine/Gate/SignalGroup/Controller `id` | 对应声明                  | 原样；Gate parent 为 path                                                             |
| `phases[].id`                             | owner-local `SignalPhase` | 原样；parent 为 controller                                                            |
| Parking area/space `id`                   | 对应声明                  | 原样；area 只是可选关系，不进入 space identity                                        |
| `frameId`                                 | `CanonicalFrame`          | 原样                                                                                  |
| Spatial edge/points                       | `LaneEdgeGeometry`        | 以 `trafficEdgeId` 绑定既有 edge；point 顺序保持                                      |

`AuthoringLane` 使用非空覆盖链的首个 current LaneEdge ID，是因为 current v0.10 没有独立
lane ID；这使 lane identity 不依赖 lane 数组重排，同时不把完整 edge chain、几何或
ordinal 写入 identity。若两个 lane 共享首 edge，输入本就违反唯一 coverage，必须在
共同 HIR 失败关闭，不能追加后缀修复。

Movement 的两个 approach key 是 current 格式缺失字段的迁移域分隔键；它们只由显式
movement ID 和固定后缀派生。派生前检查长度；超过有效 `SingleStringBytes` 返回
`LF-COMP-CURRENT-DERIVED-KEY-TOO-LONG`，不得截断或哈希替换。它们不伪装为 LaneEdge
引用；真实 entry/internal/exit topology 仍只来自 ManeuverPath。

Traffic/Spatial 数字先保留 current wire 的 `f64`/整数词法；compiler 私有降阶调用现有
共同构造/语义约束。Spatial 坐标在其真实 axis span 上检查有限性与
`[-16_384, 16_384]`，再转为 canonical `f32`；不让 source 包依赖 Spatial 类型。
完整 edge coverage、长度绑定、连接端点、owner tree、路口内部 edge、信号/等待区、
Access、Parking、route occurrence 等全部由现有共同 HIR/MIR/LIR 裁决。

Manifest 不产生领域声明，但它必须进入三文档描述符和源映射文档表。current 场景不
虚构三个逻辑模块、Traffic→Spatial import edge 或 Core-shaped intermediate object。

## 12. 原子事务与内存生命周期

`add_current_source` 的唯一事务为：

```text
borrowed compiler-owned input
  -> derive exact remaining limits
  -> strict source validation and pairing
  -> consume capability into private CurrentImportModule
  -> common prepare_admission
  -> commit_admission
```

builder 的 modules、namespace index、document index 和 totals 只在最后一步改变。任何
source、lowering、semantic 或 common admission 错误都释放 strict capability、位置和
候选，不留下部分模块或计数。输入切片在调用返回后即可释放；成功 builder 只拥有移动/
受检复制后的语义字符串、三份 origin、摘要、长度和 `SourceSpan`。

字符串、记录、引用、关系、几何和来源位置均只计数一次。允许把 owned String/Vec 从
source DTO 按值移动到私有 module；禁止 clone 完整 DTO、来源全文、所有字符串或几何点，
禁止第二次 SHA-256、第二次 JSON 文档解析、第二次资源枚举或第二次规范排序。

## 13. 资产审计与等价矩阵

G2/G3 必须生成机器可读资产清单，至少含仓库相对路径、Git blob、原始字节数、SHA-256、
格式版本、pairing 状态、迁移结论和结构化失败原因。当前冻结分类为：

| 仓库资产                                                      | 预期分类                                                                   |
| ------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `v0.1-signalized-corridor.scenario.json` 及其 Traffic/Spatial | paired-success；完整静态、pose、行为证据                                   |
| `v0.1-campus.scenario.json` 及其 Traffic/Spatial              | paired-success；静态/空间与确定性证据                                      |
| `v0.10-parking-signals-baseline.laneflow.json`                | `unpaired-traffic-only`；继续通过 Traffic-only Data，不提交三文档 importer |
| `v0.10-multi-gate-waiting-zone.laneflow.json`                 | `unpaired-traffic-only`；继续作为 current oracle，失败清单不得称格式无效   |
| 仓库 v0.2–v0.9 历史 Traffic                                   | `unsupported-current-version`；不可喂给 v0.10 importer                     |

`LF-COMP-CURRENT-EQUIV-v2` 在现有 v1 独立投影基础上增加真实 #297 importer 路径，冻结：

- paired source → compiler → LIR → integration projection 与 current production loader 的
  完整静态契约逐字段相同；
- signalized corridor 66 条 edge 的起点/中点/终点 pose 相同；
- 同一车辆/route/profile 运行 1,000 个 20 ms tick，状态与全部 Core event 经 external
  ID 映射后逐 tick 相同；
- 输入制品数组重排、Traffic/Spatial 中无语义集合重排和新进程 HashMap seed 不改变 LIR
  语义指纹、StableId 或 source mapping；有序 route、lane chain、corridor elements、
  phase program 和 point sequence 不得被误当无序集合；
- source map 的 Manifest 空声明文档仍存在；抽样覆盖三文档的 declaration、reference、
  owner-local relation、geometry axis 和派生 owner；释放原始 bytes 后仍可查询 origin；
- production-compatible 长 ref、超过 16 个唯一额外制品、唯一额外 payload 和 128 字节
  current ID 保持现有接受；相同输入在 strict 超限时于增长前返回资源诊断；
- 每个资源维度执行边界、边界加一、先加其他模块、加入顺序变形、失败不污染和重试；
- v1 compile profile 在读取/哈希/解析前以 `SourceDocumentCount` profile-incompatible
  失败，v2/后继显式多文档 profile 才能导入。

## 14. 性能收益、影响与实现代价

收益是消除 Traffic/Spatial/Manifest 的双 DTO、双版本判断和双配对权威，让资产迁移
直接复用唯一 compiler semantic pipeline，并把恶意或意外超大 current JSON 的风险限制
在 builder 的现有资源账本内。三个文档的来源身份和真实位置使差异、失败清单和后继
迁移审计不再依赖 current 数组下标与 LIR ordinal 偶然相同。

生产影响必须受控：`laneflow-data` 多一次 Manifest 文档摘要，但 Traffic/Spatial 摘要
仍各一次；production 不建立位置表或 strict lookup，不增加制品数/ref 拒绝条件。共享
泛型 DTO 会增加 source 包代码复杂度，`CaptureLocations` 的 token-local raw decode、换行
索引和位置槽会提高离线 strict 解析常数，但它们默认不进入 Runtime，且换来真实定位与
增长前上限。独立 source crate 还增加一个 unpublished 内部边界和迁移期维护成本；该成本
由阶段 8 后明确退役抵消，而不是形成永久兼容层。

G2 基准必须分别报告：production-compatible 场景加载基线/候选；strict Manifest、Traffic、
Spatial 解码；SHA-256/换行索引；位置冻结；current lowering；共同 admission；完整 compile。
不得把 strict 解析成本归入 #315 admission regression，也不得只报告成功结果 live bytes。
每份原始文档的 parse 和 SHA-256 调用次数必须由测试计数器证明为 1。

## 15. G2 实施切片

G1 Pass 后按以下顺序落地，任一切片都不能提前写入 Runtime：

1. 新建 `laneflow-current-source`，迁移 wire DTO、版本/摘要/配对与 Traffic-only 能力；
   用兼容矩阵证明 `laneflow-data` accepted set 未收窄后删除 Data 的重复 DTO/配对实现。
2. 增加 strict profile、增长前 visitor、static location policy、换行索引、位置闭合集合和
   source 自身边界测试。
3. 在 compiler 默认关闭特性下增加 `CurrentSourceArtifact`/`CurrentSourceInput`、余额派生、
   current 私有降阶、三文档描述符/源映射和原子 admission；补 compile-fail/API/DAG 测试。
4. 新建只依赖 compiler 的薄 `laneflow-current-import`，实现批量调用、机器可读资产报告和
   失败清单；文件读取、路径解析和输出写入只在该宿主工具层。
5. 完成 `LF-COMP-CURRENT-EQUIV-v2`、release 性能/内存、全 workspace CI、外部审阅、G3/G4
   证据；#294 只有在 paired success 与显式失败清单完整后才能消费删除前置。

## 16. G1 冻结清单

- [x] crate DAG、默认关闭特性、能力可见性和退役条件闭合；
- [x] `CurrentSourceArtifact::new`、`CurrentSourceInput::new` 与唯一 builder 入口精确；
- [x] production-compatible / strict / Traffic-only 三条能力及不可转换边界精确；
- [x] 一个逻辑模块、固定 namespace、三文档键、来源语言和逐文档 origin 精确；
- [x] 单一 DTO/摘要/配对权威、静态位置策略、额外制品语义和失败顺序精确；
- [x] `LF-CURRENT-SOURCE-P100-IMPORT-v1` 全部固定值、builder 余额派生和 live 事务精确；
- [x] Manifest/Traffic/Spatial 字段与记录位置闭合集合精确；
- [x] current external ID、迁移派生 key、owner/coverage/geometry 降阶精确；
- [x] repository asset 分类、等价/确定性/资源/兼容/性能矩阵精确；
- [ ] 本地全面审阅、当前 exact-head 外部 clean review 与 #297 `G1 Pass` 尚未完成。

本文件处于 Review；勾选设计内容表示候选已写全，不表示治理 Gate 已通过。只有 #297
追加 exact-head `G1 Pass` 后，本文件才能改为 Accepted 并授权 G2。
