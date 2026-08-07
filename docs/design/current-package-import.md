# 当前 Traffic/Spatial 包迁移导入前端

**文档状态**: Accepted（#297 G1 Pass；未授权 G2）<br>
**最后更新**: 2026-08-08<br>
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

下图只表达 LaneFlow 项目内包的正常依赖；箭头表示左侧正常依赖右侧：

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
`laneflow-current-import` 唯一的 LaneFlow 项目内直接依赖是启用该特性的 compiler；为了
序列化机器报告、计算清单摘要和表达宿主工具错误，它另以正常依赖显式允许第三方支撑包
`serde`、`serde_json`、`sha2` 与 `thiserror`。这组依赖不提供 current 领域对象或 compiler
旁路。`serde_json` 在 Cargo 清单使用兼容版本要求，精确解析版本只由仓库提交内唯一的
`Cargo.lock` 锁定；当前 G1 基线解析为 `1.0.151`，报告生成、已知向量和证据命令必须使用
`--locked`。不得再用清单 `=version` 建立第二个解析事实源。Cargo metadata、Cargo 图和
compile-fail 测试必须同时证明 importer 的 LaneFlow 直接依赖只有 compiler、第三方直接
依赖不超出上述白名单，并且 importer 不能直接命名 `laneflow-current-source`、受检包、严格
配置或 compiler 私有模块构建器。任何后继锁文件更新若改变报告字节，必须由报告已知向量
失败关闭，并在重新生成证据前显式审阅差异。

G2 的 importer Cargo 清单必须使用与下列等价的直接支撑依赖配置；`serde_json` 不依赖从
compiler 传递进来的可见性，`sha2` 关闭默认特性以沿用仓库现有摘要边界：

```toml
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
sha2 = { version = "0.11", default-features = false }
thiserror = "2.0"
```

`laneflow-data` 在 #297 中改为消费 production-compatible source 能力，再继续拥有
current Core/Spatial 规范化。它不得保留第二份 Traffic/Spatial/Manifest wire DTO、版本
判断、摘要或配对实现。Traffic-only Data 入口消费 source 包的独立 Traffic-only
production-compatible 能力，不强制虚构 Manifest 或 Spatial。

退役分两步：#294 生产切换时删除 `laneflow-data` 到 current Core/Spatial 的运行时 JSON
路径；离线迁移入口继续保留到以下条件全部满足：#294 已取得 G4、版本化资产审计报告
覆盖仓库资产清单与已发布资产空清单且没有未处置项、包含新 Runtime 的首个公开版本
已经发布并在发布说明中完成最后迁移通知。#297 Owner 必须在 #297 G2 前建立专用 cleanup
Issue，记录上述触发条件、删除版本和 Cleanup owner；该 Issue 删除
`current-v0_10-import`、`laneflow-current-import`、`laneflow-current-source`、current
输入/查询 API、`CurrentSourceDocumentRole` 以及
`SourceLanguage::CurrentTrafficSpatialV0_10` 与其 `as_str()` 分支。不得仅因 Runtime
已切换便先删除唯一离线迁移入口，也不得把未定义的“保留期”变成永久兼容承诺。

该专用 Issue 不能复用负责生产切换的 #294，也不能复用 #297 本身。#297 G2 开工评论必须以
append-only `current-import-cleanup-authority:v1` 结构化记录冻结专用 Issue 的规范 URL、GitHub
Issue Node ID 与 Cleanup owner 规范登录名；改变任一值都必须回到 G2 追加新的授权记录，不能编辑
旧评论。G2 实现随后把同一元组和 G2 evidence permalink 写入固定路径
`docs/reference/current-import-cleanup-authority-v1.json`。该文件是后续资产报告的可信清理责任
输入，不由报告生成器、报告内容或验证时的可变 Issue assignee 反向决定。

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

#[derive(Clone, Copy, Debug)]
pub struct CurrentImportProvenance<'a> { /* private */ }

impl<'a> CurrentImportProvenance<'a> {
    pub const fn new(
        importer_build_id: &'a str,
        importer_options_digest: [u8; 32],
        provenance: &'a str,
    ) -> Self;
}

impl<'a> CurrentSourceInput<'a> {
    pub const fn new(
        manifest_bytes: &'a [u8],
        manifest_display_source: Option<&'a str>,
        artifacts: &'a [CurrentSourceArtifact<'a>],
        import_provenance: CurrentImportProvenance<'a>,
    ) -> Self;
}

impl CompilationUnitBuilder {
    pub fn add_current_source(
        &mut self,
        source: CurrentSourceInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle>;
}
```

一个生命周期参数同时约束切片、元素及导入来源沿袭，防止 importer 用短生命周期元数据拼出长生命
周期输入。`display_source` 是未认证的显示/审计字符串，可以是仓库相对路径、资产键或
宿主标签；它不参与摘要、标识或发布真实性。`CurrentImportProvenance` 同样只借用调用方声明，
其构造器不验证声明真实性；`add_current_source` 必须在读取、哈希或解析来源文档前，使用现有
`SourceModuleHeader` 的可见 ASCII、非空、`SingleStringBytes` 与 compiler-controlled live
bytes 规则验证并复制 `importer_build_id` 和 `provenance`，并原样保存固定宽度的
`importer_options_digest`。失败不保留部分模块或字符串。

官方 `laneflow-current-import` 必须从自身实际构建元数据和本次转换选项构造该值，不得接受会
把任意宿主伪装成官方构建的 CLI 覆盖，也不得继续使用 `laneflow-current-import/v1` 或
`current-package-migration` 这类工具/用途常量冒充实际构建与转换沿袭。直接调用 compiler 的
其他宿主必须登记自己的构建、选项与沿袭，因此不同工具不会得到相同模块审计元数据。这些字段仍
只是调用方登记的审计声明，不是发布信任锚；其真实性由后继外部描述符/验证收据或宿主认证资产链
绑定，不能据此跳过内容摘要或独立验证。

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
pub fn validate_scenario_strict<'a, I>(
    manifest: CurrentDocumentInput<'a>,
    artifacts: I,
    limits: &CurrentSourceLimits,
) -> Result<ValidatedCurrentImportBundle, CurrentSourceError>
where
    I: Clone + ExactSizeIterator<Item = CurrentArtifactInput<'a>>;
```

Source 输入类型与 compiler 输入类型有意不同：前者是 source 包的跨包实现入口，后者
保证 importer 的 LaneFlow 依赖闭包只需 compiler。compiler 在 `add_current_source` 栈内把
`CurrentSourceArtifact` 借用切片惰性映射为 `CurrentArtifactInput` 迭代器并直接传入 source；
转换只复制每项的借用指针/长度元数据，不复制 payload，也不分配中间 `Vec`。

```rust
let artifacts = source
    .artifacts
    .iter()
    .copied()
    .map(to_current_artifact_input);
let validated = validate_scenario_strict(manifest, artifacts, &limits)?;
```

`to_current_artifact_input` 是 compiler 包内私有的纯借用转换函数；使用函数项而非捕获状态的
closure，使该 `Map<Copied<slice::Iter<_>>, _>` 明确满足 `Clone + ExactSizeIterator`。

严格入口的无分配预检只由 source 拥有。source 首先完整消费 `artifacts.clone()`，观察到
第 17 项时在任何与该项相关的增长前拒绝，并逐项检查非空引用、单个引用、引用总字节和显示来源
等 source 专用硬上限；
它不得只信任 `ExactSizeIterator::len()`。两次遍历都固定为“实际资源失败优先”：一旦实际观察到
第 17 项、超长 ref 或其他 source 硬上限越界，立即返回对应资源诊断；只有本次遍历在全部硬上限内
正常结束后，才比较数量契约。克隆预检以初始 `len()` 为 `expected_len`、实际产生数为
`actual_len`；只有两者相等，source 才按该受检数量向事务账本申请 lookup backing 并消费原
迭代器。消费原迭代器时必须在每次 capacity 增长前重复计数和相关硬上限检查；若遍历在上限内
完成但最终数量与克隆预检数量不同，则以后者为 `expected_len`、本次数量为 `actual_len` 返回
source 结构化输入契约诊断。compiler 不得出现数值 `16`、`256`、`1,024` 或对应 source 诊断的
镜像检查，只负责构造借用迭代器、传入调用点动态余额并消费原子结果。

前两条只供 current production façade；第三条因 Rust 不存在 friend crate 而必须跨包
可见，但官方路径只由 compiler 调用。任意外部程序自行依赖 source 包并调用严格入口
属于 caller-owned 工作；它不能把结果提交给 compiler，也不在 compiler 资源保证内。

`ValidatedCurrentTrafficPackage`、`ValidatedCurrentSourceBundle` 和
`ValidatedCurrentImportBundle` 均无公开字段、`Default`、Serde 实现或裸构造器。
production scenario 能力原子拥有三份已验证 wire 内容、精确文档摘要和 Manifest
配对；它不包含逐文档 compiler origin 或位置表。strict 能力不可分地拥有相同来源包、
三个逐文档 origin、受限位置数据和精确资源用量。production 能力不能升级为 strict
能力，strict 能力也不能降级后再提交。

跨包消费固定为 capability 上的借用 accessor 与消费型 `into_parts(self)`；返回的
`CurrentTrafficParts`、`CurrentSourceParts`、`CurrentImportParts` 字段仍私有，只提供
逐项借用 accessor 和 owned iterator。它们因 Rust 跨 crate 可见性必须使用
`#[doc(hidden)] pub`，但 source 包保持未发布；没有 `Clone`、Serde、从裸 parts 反向构造 capability 或
parts→compiler 提交入口。Data 与 compiler 必须通过这些视图消费同一 DTO，不得各自再
反序列化原始 JSON。具体 wire record 类型可以保持私有，G2 不得把整棵 DTO 改成公共
字段以规避消费接口。

## 6. 文档身份、来源语言与来源记录

三文档共同形成一个无导入边的逻辑模块：

| 项目                         | 精确值                                                    |
| ---------------------------- | --------------------------------------------------------- |
| `authoringNamespaceId`       | `current/v0.10`                                           |
| `SourceLanguage` 新值        | `CurrentTrafficSpatialV0_10 = 2`                          |
| `SourceLanguage::as_str()`   | `current-traffic-spatial-v0.10`                           |
| `frontendVersion`            | `1`                                                       |
| Manifest `sourceDocumentKey` | `current/v0.10/manifest`                                  |
| Traffic `sourceDocumentKey`  | `current/v0.10/traffic`                                   |
| Spatial `sourceDocumentKey`  | `current/v0.10/spatial`                                   |
| imports                      | 空集合                                                    |
| `generatorBuildId`           | `CurrentImportProvenance::new` 的实际 `importer_build_id` |
| `frontendOptionsDigest`      | `CurrentImportProvenance::new` 的实际选项摘要             |
| `parametersAndInputsDigest`  | Manifest 原始字节 SHA-256                                 |
| `randomSeed`                 | `None`                                                    |
| `provenance`                 | `CurrentImportProvenance::new` 的本次转换来源沿袭         |

稳定文档键不含宿主路径、`artifactRef`、输入数组下标或内容摘要。固定 namespace 意味着
一个编译单元至多接收一个 current 场景；第二次加入由现有重复 namespace 诊断原子拒绝。
`frontendVersion=1` 只标识 current 前端线格式；它不能替代具体 importer build。官方 importer
当前没有影响转换语义的可选开关时，`importer_options_digest` 使用
SHA-256(`LFCURRENT-IMPORT-OPTIONS` + little-endian `1_u32`)；一旦增加转换选项，必须按提升后的
封闭选项记录重新派生，不能沿用该无选项摘要。宿主文件路径、报告输出位置等不影响模块转换的工具
选项不得混入该摘要。

`SourceLanguage::CurrentTrafficSpatialV0_10` 枚举变体及其 `as_str()` match arm 都必须标注
`#[cfg(feature = "current-v0_10-import")]`。默认特性构建的 public API 与 rustdoc 不得出现
该变体；启用迁移特性时其 `repr(u16)` 值固定为 `2`。`SourceLanguage` 已经是
`#[non_exhaustive]`，条件变体不增加下游穷尽匹配承诺。

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

为在一次根文档遍历中保留现有的 syntax → `formatVersion` 头部 shape → unsupported version
→ 其他 shape 优先级，顶层与嵌套容器使用手写 `DeserializeSeed`/visitor：JSON 词法或结构无法
安全继续时立即返回原始 syntax 错误；每份文档的 `formatVersion` 是头部闸口，其缺失、显式
`null`、非字符串或重复 occurrence 立即返回 `JsonShape`，重复时不得选择第一个或最后一个值
继续做版本裁决。只有恰好一个合法字符串 occurrence 才参与 unsupported version 判断。
其他可安全跳过的类型、unknown field 和缺字段问题只保存首个有类型 shape 候选并继续消费当前
JSON 值；完整 JSON 有效且头部闸口通过后先裁决 unsupported version，再返回其余 shape 候选。
production 只保存首个候选；strict 可在诊断上限内保存多个候选。visitor 不得构造
`serde_json::Value` 或 Serde `Content` 树，也不得捕获或重放整个文档。

两种策略都对固定 current schema 的单个 record/point token 借用 `&RawValue`：根 visitor
消费该 token 一次以验证 JSON 边界并取得真实 `[start, end)`，随后在原切片上至多解码该
record 一次；嵌套 record 采用同一规则。`NoLocations` 只为 version-before-shape 和准确的
单故障错误锚点使用该范围，成功时不保留位置；`CaptureLocations` 另把第 10 节闭合集合封存
到 packed table。两者都不缓存 owned JSON 子树、不重新扫描整份文档，也不在解析后遍历
序列重建字段路径。固定 current schema 的 record 嵌套深度是常数；production 与 strict
基准必须分别报告 record-token replay 的字节数与时延。
`laneflow-current-source` 为此显式启用 serde_json 的 `raw_value` feature，不增加第二个 JSON
parser 依赖。该有界 replay 是 `compiler-foundation.md` 第 2.3 节唯一允许的局部例外，不能
扩展到根文档或任意 ignored container。

`CaptureLocations` 对需要定位的标量继续使用借用 `RawValue` 新类型：先以原始 token 长度
执行字符串/数值增长前检查，再在同一字段 visitor 内完成词法解码，并把 token 在原始输入
切片中的 `u32` 起止 byte offset 记入有类型位置槽。每份文档的根 deserializer 和 SHA-256
各执行一次；SHA-256 的同一次线性扫描始终记录首个与最后一个非空白 byte，strict 另收集
换行 byte offset。完整 JSON 成功且根值为 object 后，首尾 offset 形成根 object 的真实
`[start, end)`，供根级 missing field 使用，不捕获或 replay 根文档。位置冻结时通过该有界换行
索引验证每个范围并封存 packed table，但不把全部条目提前膨胀为四个行列 `u32`。strict
capability 保留 packed byte range 与换行索引；compiler lowering 按项转换为现有
`SourcePosition { line, column }`。packed table、换行索引和不能原样转移的 DTO container
capacity 在其 backing allocation 实际 drop 前保持完整 source charge；按文档或整表完成
lowering 并 drop 后才一次性解除。只有最终保留的最多 16 条 source issue 会在返回错误前物化
`CurrentSourceSpan`。production 的 `NoLocations` 不建立换行索引、不携带成功路径位置字段，
也不构造 `CurrentSourceLocationTable`；只有返回延迟 shape 错误时，才对原始字节执行一次
allocation-free 前缀扫描，把唯一错误 anchor 转为一基行列。该错误路径投影不是第二次 JSON
解析，不能用于重建 DTO 或字段路径。

延迟 shape 候选的锚点固定为：类型错误、explicit null、unknown/duplicate field 使用该
field 的 value token；missing field 使用所属 record token；根级 missing field 使用根
object token。record replay 内返回的 `serde_json::Error` 行列先转为相对 byte offset，再
加 record 的全局 `start`，不能把局部行列直接暴露为文档位置。

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
2. source 通过上述克隆迭代器执行 allocation-free 预检，检查实际制品数、每个 ref、ref
   总字节，以及 Manifest 和每个输入制品的单项 display source；compiler 不镜像这些规则；
3. 先检查 Manifest 实际字节的 source 单文档硬上限，再以入口即已知且必然被选中的 Manifest
   字节长度作为下界，依次检查 compiler `SourceBytesPerModule` 和剩余 `SourceBytesTotal`；任一
   失败都在 Manifest SHA-256、换行索引、DTO 或其他按输入规模分配前返回，随后才计算一次
   Manifest SHA-256/换行索引并有界解析；该下界检查不预留、不扣账；
4. 按 Traffic descriptor 的非空 ref → media type → portable size → digest 词法、Spatial
   descriptor 的相同顺序、两者 ref 冲突的既有优先级完成 Manifest 语义验证；
5. 对调用方 ref 集合执行非空/全集合唯一检查并定位两个目标，再检查 Manifest 与被选中
   Traffic/Spatial 三份 display source 的总字节；未引用制品的 display source 不进入
   origin 或该总量；
6. source 在 Manifest 绑定后唯一计算 `selected_source_bytes = manifest + selected Traffic +
   selected Spatial`；在 Traffic/Spatial 摘要或 DTO 分配前依次检查声明长度、实际长度、
   source hard cap、compiler per-module 余额和 compiler total 余额。该完整值复核不继承或重复
   计算第 3 步的 Manifest 下界为已提交量；未引用制品 payload 不进入该值；资源失败允许提前，
   但非资源 size mismatch 只记录到下一步裁决；
7. 按 Traffic actual size → Traffic SHA-256/digest → Spatial actual size → Spatial
   SHA-256/digest 的既有优先级验证；每份摘要扫描同时建立其 strict 换行索引；
8. 按 Manifest → Traffic → Spatial 顺序有界解析，同步累计 wire、语义、位置和 live
   资源；
9. 返回一个不可拆的 strict 能力；compiler 随即降阶并执行共同候选复核；
10. 全部检查成功后调用现有 `commit_admission`，否则释放候选且 builder 不变。

production 在 source 层保持现有可观察的非资源失败顺序：Manifest syntax →
`formatVersion` 头部 shape → unsupported version → 其他 Manifest shape → Traffic descriptor →
Spatial descriptor → conflicting ref → provided refs → Traffic
size/digest → Spatial size/digest → Traffic wire → Spatial wire。只有 strict 可以把新增的
资源失败提前。source 原子成功后，Data 仍按 Traffic → Spatial 执行 current
Core/Spatial 规范化；兼容承诺冻结 accepted set、公共错误 variant、document、path 与 JSON
category，但不承诺旧 Serde loader 的精确 `line`/`column` 数值。新路径统一返回本文第 10 节
定义的准确规范锚点（canonical anchor）；不得为复刻旧游标位置保留第二套旧位置或第二个 DTO。
一个输入同时含 source 错误与后续 domain 错误时，两者的相对首错优先级同样不冻结。

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
- 131,072 个序列项是 importer 自身的防御性 hard cap，不声称是所有 compiler-admissible
  current 语法组合的数学包络；仓库最大 paired 场景有 6,352 个 sequence item 和 8,036
  个 JSON value，分别保留 20.63 倍和 32.62 倍余量。strict 可以在共同 compiler 维度尚有
  余额时先拒绝病态 source，production-compatible accepted set 不受该 hard cap 约束；
- 24 MiB source live 上限按 58,387 个 128-byte record slot、131,072 个 16-byte sequence
  slot、262,144 个 16-byte location slot、542,741 个 owned string byte、最坏每个来源
  byte 一个 `u32` 换行 offset，再加 2 MiB lookup/control reserve 求和为 18,575,849
  字节；剩余 6,589,975 字节吸收高于 record 最低记账额的实际 capacity、小容器与对齐，
  但所有实际 capacity 仍逐次精确计费。

16-byte location slot 不是对未定义 Rust struct 大小的假设。实现使用两个私有 8-byte
word slot：`span_words: [u32; 2] = [start, end]` 与
`key_words: [u32; 2] = [record_ordinal, packed_tag]`。`packed_tag` 的低 16 bit 是 field key、
随后 8 bit 是 record kind、随后 2 bit 是 document role、最高 6 bit 必须为零；record 与
field 闭合枚举在 pack/unpack 时验证，位置槽不保存字符串或指针。16-byte sequence slot
同样固定为 `[u32; 4] = [owner_record_ordinal, packed_field_kind, start, len]`，只索引 record/
string arena。G2 必须用 `size_of`、最大 capacity、pack/unpack 已知向量和 transaction 峰值
测试证明两种 slot 每项精确 16 byte；若实现不能满足该布局，必须回到 G1 提升 profile ID
或重算预算，不能静默提前拒绝已冻结边界 workload。

各 hard limit 是独立拒绝上界，不承诺其笛卡尔积全部可同时达到；24 MiB live 维度可以在
record/location/sequence 各自 count 尚有余额时先失败。128-byte record slot 是逐 record
最低记账额，实际 typed arena/owned capacity 大于该值时按实际请求字节计费。该规则保证
失败关闭，不用低估内存换取表面上的最大 count；G2 边界 workload 必须同时证明已冻结
repository/published P100 资产不会被 live 维度意外提前拒绝。

profile 提升必须携带 repository/published asset 失败清单、上述算式重算和 release 峰值
证据；不得只因某个外部输入超限便原地放宽 v1。

`WireRecordCount` 计 object 记录和有语义的 point 三元组，不重复计 object 的字段；
`SequenceItemCount` 计每次数组出现。`JsonValueCount`、深度、字符串和序列在 visitor 增长
前饱和累计。哈希表请求字节沿用共同接入的保守八桶/控制字模型；Vec、Box、String 和
换行/位置表按请求 capacity 计入 live bytes，不能用成功后的 len 冒充峰值。

current 模块的固定共同需求与空 builder 余额分开表达，不能把需求量误写成 profile 上限：

| 固定需求              | 精确值 |
| --------------------- | -----: |
| `ModuleCount`         |      1 |
| `SourceDocumentCount` |      3 |
| `ImportEdgeCount`     |      0 |

| `LF-COMP-P100-INITIAL-v2` 共同维度          |     空 builder 上限 |
| ------------------------------------------- | ------------------: |
| `ModuleCount` / `SourceDocumentCount`       |       `522 / 1,566` |
| `ImportEdgeCount`                           |               1,032 |
| `SourceBytesPerModule` / `SourceBytesTotal` | `542,741 / 542,741` |
| `DeclarationCount` / `SymbolCount`          |   `11,265 / 11,265` |
| `TypedAstRecordCount`                       |              58,387 |
| `ReferenceCount`                            |              37,920 |
| `RelationOccurrenceCount`                   |              10,032 |
| `IdentityFieldOccurrenceCount`              |              29,184 |
| `RouteOccurrenceCount`                      |               1,920 |
| `ManeuverGateCount` / `WaitingZoneCount`    |     `2,304 / 1,536` |
| `GeometryPointCount`                        |              22,368 |
| `StringItemCount`                           |              36,894 |
| `SingleStringBytes` / `TotalStringBytes`    |      `53 / 991,537` |
| `DiagnosticCount`                           |                  16 |
| `CompilerControlledLiveBytes`               |          43,269,120 |

compiler 只把 source 验证实际需要的动态余额传入 source 包；共同 lowering 维度不跨 crate
镜像：

```rust
#[doc(hidden)]
pub struct CurrentCompilerBudget {
    pub max_source_bytes_per_module: u64,
    pub max_source_bytes_total: u64,
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

该构造器要求 diagnostic 和 import transaction live budget 非零；source byte 零余额仍是
可构造的有效上限，但 source 必须在 Manifest 单文档硬上限检查后，立即以 Manifest 实际字节
长度作 compiler 局部/累计余额下界检查，并在 Manifest 哈希、换行索引、DTO 或其他按输入规模
分配前失败。实现必须分别保存固定 source hard cap 与每个 compiler 动态余额及其
`CurrentCompilerBudgetDimension`，不能预先取最小值而丢失失败来源。它不接受 profile ID
字符串、source 硬上限或其他共同 compiler 维度。compiler 必须先
自行确认所选 `CompileLimits` 显式支持 1 个模块和 3 份剩余文档，才可构造 budget。其他
crate 即使直接依赖 source 包并构造该值，也不能把其结果提交给 compiler。

有效值不是该表的过期快照。`add_current_source` 持有 `&mut self` 后，用饱和/受检减法从
当前 `AdmissionTotals` 和 builder live bytes 派生 source bytes、diagnostic 与 transaction
live 余额。source 先按 Manifest 单文档硬上限 → compiler `SourceBytesPerModule` → 剩余
`SourceBytesTotal` 的固定优先级检查 Manifest 字节下界；配对后再按三文档单项、source profile
总量、compiler 单模块余额和累计余额的相同来源优先级复核完整 `selected_source_bytes`，并把
该唯一完整值随不可伪造 capability 交给 compiler。前置下界检查不是资源提交，compiler 只记录
最终完整值。

共同维度的唯一计数权威是 compiler 私有 current module builder 的受预算构造操作：

| 维度                                                              | 唯一增长点                                                                                                     |
| ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `ModuleCount` / `SourceDocumentCount` / `ImportEdgeCount`         | compiler preflight 固定提交 `1 / 3 / 0`，source 不重复计数                                                     |
| `SourceBytesPerModule` / `SourceBytesTotal`                       | source 先检查 Manifest 字节下界，配对后唯一产生完整 `selected_source_bytes`；compiler 只提交完整值且不重新求和 |
| `DeclarationCount` / `SymbolCount`                                | 第 11 节每次成功发射一个共同声明时各增加 1                                                                     |
| `TypedAstRecordCount`                                             | 模块头和共同 typed record/point 构造器成功保留一项时增加 1                                                     |
| `ReferenceCount`                                                  | 每次共同 typed reference 构造器成功保留一项时增加 1                                                            |
| `RelationOccurrenceCount` / `IdentityFieldOccurrenceCount`        | 对应共同 occurrence 构造器成功保留一项时增加 1                                                                 |
| `RouteOccurrenceCount` / `ManeuverGateCount` / `WaitingZoneCount` | 对应共同 occurrence 或实体构造器成功保留一项时增加 1                                                           |
| `GeometryPointCount`                                              | 每个 Spatial point 成功转为 canonical point 时增加 1                                                           |
| `StringItemCount` / `TotalStringBytes`                            | 每次 destination string 或 origin string 首次进入模块所有权时增加 1 / 原始 UTF-8 字节数                        |
| `SingleStringBytes`                                               | 每个 semantic external ID、固定 document key 和 derived key 在 destination 分配前检查                          |
| `DiagnosticCount`                                                 | 现有 compiler/source collector 的共享保留上限；每个最终保留 issue 只在所属 collector 计 1                      |
| `CompilerControlledLiveBytes`                                     | `add_current_source` 唯一 transaction ledger 在每次 capacity/ownership transfer 前计费                         |

这些构造器在同一 lowering 循环中累计模块资源计数，并把同一结果交给
`prepare_admission`；source 包不预估 declaration/reference/relation 等 lowering 结果，
compiler 也不为 source bytes 或 admission 再遍历输入/记录。三份 document key、role、artifact ref 与
display source 分别计入 `StringItemCount`、`TotalStringBytes` 和 live bytes；artifact ref
与 display source 不属于 `SingleStringBytes`，但仍受 source 专用 256/1,024 byte 上限。
`importer_build_id` 与模块级 `provenance` 沿用 `SourceModuleHeader` 的
`SingleStringBytes`/可见 ASCII 规则，只按实际复制字节计 compiler-controlled live bytes，不
重复计为来源文档字符串；32 字节选项摘要不产生按输入规模分配。这些验证后复制与固定宽度
摘要的物化存储在读取、哈希或解析任何来源文档前完成，并立即计入事务账本的
materialized upfront charge，在整个事务期间保持，直到原子提交转为模块描述符的
destination charge 或失败时随事务释放，不作废"增长前失败"不变量。

`max_import_transaction_live_bytes` 精确等于调用点剩余的
`CompilerControlledLiveBytes` 扣除全部 materialized upfront charge 后的余额，不另设无限值；
source 与事务内任何其他 candidate allocation 只能在该扣减后余额内增长。解析期同时受
24 MiB source 峰值和该余额约束；降阶时使用同一事务账本。只有底层 allocation 原样移动到
destination 的 String/Vec 才能把 source charge 转为 destination charge；packed location、
换行索引、owned iterator backing 与需要重新分配的 DTO container 在实际 drop 前保持
source charge。任何新 capacity 请求前检查
`committed_builder + materialized_upfront + full_live_source_backing + destination_candidate`；
按文档或整表完成转换并 drop backing 后才一次性解除对应 charge。共同 admission 前
strict bundle、位置表和 source-only iterator backing 必须已释放。

G2 transaction 峰值测试至少固定四个观测点：最大 packed table 刚冻结、转换一半、全部元素
已消费但 owned iterator/backing 尚未 drop、drop 后。每点都比较 ledger、实际 capacity 与
destination `SourceSpan` 增长；只有第四点允许解除整块 source charge。

语义字符串的 53 字节上限是 compiler profile，不改变 Traffic schema 的 128 字节接受
域。production-compatible Data 路径继续接受 schema/current loader 已接受的值；strict
source 可以在其有界 DTO 中保留该值，compiler lowering 必须在 destination 分配前以现有
`LF-COMP-RESOURCE-LIMIT`、dimension=`SingleStringBytes` 失败。禁止截断、哈希替代、
改写 external ID 或隐式提升 compiler profile。

## 9. 结构化资源诊断

Source 包的失败面是非空 issue bundle，而不是同时承担“单个问题”和“最多 16 个问题”
两种含义的裸枚举：

```rust
pub struct CurrentSourceError {
    issues: Box<[CurrentSourceIssue]>,
}

impl CurrentSourceError {
    pub fn issues(&self) -> &[CurrentSourceIssue];
    pub fn into_issues(self) -> Box<[CurrentSourceIssue]>;
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CurrentDocumentRole {
    Manifest,
    Traffic,
    Spatial,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CurrentArtifactRole {
    Traffic,
    Spatial,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CurrentSourcePosition {
    line: u32,
    column: u32,
}

impl CurrentSourcePosition {
    pub const fn line(self) -> u32;
    pub const fn column(self) -> u32;
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CurrentSourceSpan {
    start: CurrentSourcePosition,
    end: CurrentSourcePosition,
}

impl CurrentSourceSpan {
    pub const fn start(self) -> CurrentSourcePosition;
    pub const fn end(self) -> CurrentSourcePosition;
}

pub struct CurrentSourceIssue {
    payload: CurrentSourceErrorPayload,
    document: Option<CurrentDocumentRole>,
    context: CurrentSourceIssueContext,
    path: Option<Box<str>>,
    span: Option<CurrentSourceSpan>,
}

#[doc(hidden)]
pub enum CurrentSourceIssueContext {
    None,
    ScenarioTraffic { artifact_ref: Box<str> },
}

#[doc(hidden)]
pub struct CurrentSourceIssueParts {
    payload: CurrentSourceErrorPayload,
    document: Option<CurrentDocumentRole>,
    context: CurrentSourceIssueContext,
    path: Option<Box<str>>,
    span: Option<CurrentSourceSpan>,
}

impl CurrentSourceIssue {
    pub const fn payload(&self) -> &CurrentSourceErrorPayload;
    pub const fn document(&self) -> Option<CurrentDocumentRole>;
    pub fn artifact_ref(&self) -> Option<&str>;
    pub fn path(&self) -> Option<&str>;
    pub const fn span(&self) -> Option<CurrentSourceSpan>;

    #[doc(hidden)]
    pub fn into_parts(self) -> CurrentSourceIssueParts;
}

impl CurrentSourceIssueParts {
    #[doc(hidden)]
    pub fn into_components(
        self,
    ) -> (
        CurrentSourceErrorPayload,
        Option<CurrentDocumentRole>,
        CurrentSourceIssueContext,
        Option<Box<str>>,
        Option<CurrentSourceSpan>,
    );
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CurrentCompilerBudgetDimension {
    SourceBytesPerModule,
    SourceBytesTotal,
    CompilerControlledLiveBytes,
}

pub enum CurrentSourceErrorPayload {
    JsonSyntax { source: serde_json::Error },
    JsonShape { source: serde_json::Error },
    UnsupportedFormatVersion { expected: &'static str, actual: Box<str> },
    EmptyArtifactReference,
    ConflictingManifestArtifactReference { artifact_ref: Box<str> },
    DuplicateProvidedArtifactReference { artifact_ref: Box<str> },
    MissingArtifact { role: CurrentArtifactRole, artifact_ref: Box<str> },
    InvalidMediaType { expected: &'static str, actual: Box<str> },
    InvalidDigest { actual: Box<str> },
    ArtifactSizeOutOfRange { actual: u64, max: u64 },
    ArtifactSizeMismatch {
        role: CurrentArtifactRole,
        artifact_ref: Box<str>,
        expected: u64,
        actual: u64,
    },
    ArtifactDigestMismatch {
        role: CurrentArtifactRole,
        artifact_ref: Box<str>,
        expected: Box<str>,
        actual: Box<str>,
    },
    ArtifactIteratorContractMismatch {
        expected_len: u64,
        actual_len: u64,
    },
    LimitExceeded {
        profile_id: &'static str,
        dimension: CurrentSourceLimitDimension,
        limit: u64,
        observed: u64,
        phase: CurrentSourceLimitPhase,
    },
    CompilerBudgetExceeded {
        dimension: CurrentCompilerBudgetDimension,
        remaining: u64,
        observed_delta: u64,
        phase: CurrentSourceLimitPhase,
    },
}

impl CurrentSourceErrorPayload {
    pub const fn stable_code(&self) -> &'static str;
}
```

`CurrentDocumentRole` 与 `CurrentArtifactRole` 都是 source 包闭合枚举；前者为
`Manifest`、`Traffic`、`Spatial`，后者为 `Traffic`、`Spatial`。compiler 使用穷尽 match
把前者转换为 feature-gated `CurrentSourceDocumentRole`，不得按整数或字符串重解释。
所有类型提供只读 accessor；只有 source 包可以构造 issue 或空检查后的 bundle。
`CurrentSourceIssueParts::into_components` 是 Data/Compiler 取走不可 Clone
`serde_json::Error` 的唯一 owned bridge；不得通过重建 JSON error 或重新解析 Manifest
绕过它。`CurrentSourceIssueContext` 只有 `None` 与 `ScenarioTraffic`；只有 scenario 路径中
已完成 Manifest 绑定的 Traffic wire/version issue 使用后者并携带 `artifact_ref`，Traffic-only
façade 和其他文档 issue 必须是 `None`，不能用两个相邻的 `Option<Box<str>>` 表达上下文与 path。
`CurrentSourceError` 永远至少有一项：立即失败形成单元素 bundle；可安全收集的候选先按
全局规范顺序维护最小 `k = max_diagnostic_count` 条，再升序冻结；它必须等价于保存全部
候选、完整排序后截取前 `k` 条，但实现只使用固定容量 max-heap/有序数组，不能按输入顺序
先截断。固定 profile 的 `k` 为 16。

`CurrentSourceLimitDimension` 闭合为第 8 节 source 专用表中的 17 个变体。
`CurrentSourceLimitPhase` 闭合为 `InputPreflight`、`ManifestDecode`、`ArtifactBinding`、
`TrafficDecode`、`SpatialDecode`、`LocationFreeze`。compiler 把 source 专用资源失败映射为
`LF-COMP-CURRENT-SOURCE-LIMIT-EXCEEDED`，保留 profile、dimension、limit、observed、
phase、document、path 和真实 span；不得改写成 JSON shape 或普通 compile limit。共同
compiler 余额失败使用独立 `CompilerBudgetExceeded`，compiler 按穷尽 dimension 映射为现有
`LF-COMP-RESOURCE-LIMIT`：per-module 直接使用 `observed_delta`；total 维度用 builder
已提交量加 `observed_delta` 重建共同 profile 的 `observed`；live 维度还必须再加
materialized upfront charge——交给 source 的余额已扣除该 charge，其 `observed_delta`
不含它，漏加会低报 `observed`，甚至出现 `observed` 未超 `limit` 却失败的矛盾诊断。
total/live 必须分开计算，不得把动态余额先与 source hard
cap 取最小值后丢失来源。结构化诊断测试必须断言：边界相等时成功、边界加一时失败且
live 维度 `observed` 恰为 `limit + 1`（含 upfront charge），失败发生在任何来源分配
增长前。

Manifest 字节下界的 compiler 预算失败使用 `ManifestDecode`，`observed_delta` 是 Manifest
实际字节长度；配对后完整三文档预算失败使用 `ArtifactBinding`，`observed_delta` 是完整
`selected_source_bytes`。前者不预留或扣减余额。两阶段都先裁决 source 自身硬上限，再按
`SourceBytesPerModule` → `SourceBytesTotal` 裁决 compiler 动态余额；若遍历输入时已实际触发
source 硬上限，则该资源诊断仍优先于后续数量契约诊断。

`ArtifactIteratorContractMismatch` 只表示跨包 `ExactSizeIterator` 的一次遍历在全部 source
硬上限内正常结束后，实际产生数量与其比较基准不一致：克隆预检比较初始 `len()`，原遍历比较
克隆预检的实际数量。遍历中实际观察到的制品数、ref 或其他 source 资源越界立即返回对应
`LimitExceeded`，不继续消费迭代器以等待契约不匹配。该错误的 document、context、path 与 span
都为 `None`，官方 compiler 适配器出现该错误即为实现缺陷。compiler 将它一对一映射为
`LF-COMP-CURRENT-SOURCE-INPUT-CONTRACT`，不得伪装为制品数超限、JSON shape 或普通编译资源失败。

Source JSON 错误保留 category、message、规范 `$` path 和真实 span。立即失败的 syntax
携带原始 `serde_json::Error`；延迟 shape 候选可以使用 `serde::de::Error::custom` 保存
Data category 与 source chain，但该对象的内部 `line()`/`column()` 不是位置事实源。
`laneflow-data` 必须新增接收显式 `span.start` 的内部构造路径，把它映射为现有
`DataError`/`ScenarioError` variant 及其 document、path、line、column 字段；禁止再次从
延迟 `serde_json::Error` 读取可能为 `0:0` 或文档末尾的位置。兼容测试冻结公共 variant、
document、path 与 category，并验证新 line/column 是第 10 节 canonical anchor；不比较旧
Serde 游标数值，也不要求嵌套 source error 的 `Display` 逐字节相同。compiler 为这些错误
分别使用 current-source syntax/shape/version/artifact-binding 诊断；共同语义错误继续
使用现有 compiler 诊断码和 canonical ordering。

资源 preflight、摘要/配对和无法安全继续解析的 syntax 错误为单一立即失败；可安全收集
的同阶段 shape/semantic 候选按
`document role → start position → stable issue code → artifact ref UTF-8 bytes → canonical path UTF-8 bytes → typed payload`
排序，`None` 在对应 `Some` 前。role、dimension 和数值字段参与 typed payload
比较；JSON message、`Display` 文本和嵌套 source 文本不参与排序或摘要。

`CurrentSourceErrorPayload::stable_code()` 以穷尽 match 返回下表固定 ASCII 值；新增 variant
必须同时扩展该表，不能退化为 `Debug`/`Display`：

| payload variant                        | stable issue code                            |
| -------------------------------------- | -------------------------------------------- |
| `JsonSyntax`                           | `LF-CURRENT-SOURCE-JSON-SYNTAX`              |
| `JsonShape`                            | `LF-CURRENT-SOURCE-JSON-SHAPE`               |
| `UnsupportedFormatVersion`             | `LF-CURRENT-SOURCE-FORMAT-VERSION`           |
| `EmptyArtifactReference`               | `LF-CURRENT-SOURCE-EMPTY-ARTIFACT-REF`       |
| `ConflictingManifestArtifactReference` | `LF-CURRENT-SOURCE-CONFLICTING-ARTIFACT-REF` |
| `DuplicateProvidedArtifactReference`   | `LF-CURRENT-SOURCE-DUPLICATE-ARTIFACT-REF`   |
| `MissingArtifact`                      | `LF-CURRENT-SOURCE-MISSING-ARTIFACT`         |
| `InvalidMediaType`                     | `LF-CURRENT-SOURCE-MEDIA-TYPE`               |
| `InvalidDigest`                        | `LF-CURRENT-SOURCE-DIGEST`                   |
| `ArtifactSizeOutOfRange`               | `LF-CURRENT-SOURCE-ARTIFACT-SIZE-RANGE`      |
| `ArtifactSizeMismatch`                 | `LF-CURRENT-SOURCE-ARTIFACT-SIZE-MISMATCH`   |
| `ArtifactDigestMismatch`               | `LF-CURRENT-SOURCE-ARTIFACT-DIGEST-MISMATCH` |
| `ArtifactIteratorContractMismatch`     | `LF-CURRENT-SOURCE-ITERATOR-CONTRACT`        |
| `LimitExceeded`                        | `LF-CURRENT-SOURCE-LIMIT`                    |
| `CompilerBudgetExceeded`               | `LF-CURRENT-SOURCE-COMPILER-BUDGET`          |

## 10. 来源位置闭合集合

`CurrentSourceLocationTable` 只保存下列位置的 packed byte range；每项均由文档角色、
有类型记录键/字段键和真实 `[start, end)` 组成，并由 capability 同时拥有的换行索引解析。
identity/owner/relation 的 range 必须与其目标键一起移动，compiler lowering 解析成
`SourceSpan` 后继续随同一 permutation 移动，不能依赖 current 数组下标与 canonical
ordinal 相同。

解析期原始范围统一为零基、半开 byte 区间 `[start, end)`；字符串 token 包含双引号，
object/array record 包含首尾 delimiter。冻结后的 `CurrentSourceSpan` 与 compiler
`SourceSpan` 使用一基行列和包含式 end；column 按 UTF-8 byte 计数，与
`serde_json::Error` 一致。只有 `LF` 增加行号；`CRLF` 中的 `CR` 属于前一行，`LF` 后首
byte 为下一行第 1 列。非空 token 的 end 取 `end - 1` 所在位置；syntax/EOF 没有完整
token 时使用 `serde_json::Error` 的一基位置构造单点 span。边界测试必须覆盖 ASCII、
多字节 UTF-8、转义字符串、`LF`、`CRLF`、空 object/array、trailing content 与 EOF。

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
现有 `LF-COMP-RESOURCE-LIMIT`，dimension=`SingleStringBytes`，并在 typed path/reason
中标明 derived approach key；不得另立看似语义错误的顶层诊断码，也不得截断或哈希
替换。它们不伪装为 LaneEdge 引用；真实 entry/internal/exit topology 仍只来自
ManeuverPath。

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
禁止第二次 SHA-256、第二次根 JSON 文档解析、第二次资源枚举或第二次规范排序；唯一允许
的局部重放是第 7 节两种策略共享的有界 record token。`NoLocations` 只保留单故障 anchor，
`CaptureLocations` 才封存完整 packed table。

## 13. 资产审计与等价矩阵

G2/G3 必须生成符合
[`current-asset-audit-v1.schema.json`](../reference/current-asset-audit-v1.schema.json) 的
`laneflow.current-asset-audit` v1 报告。仓库资产清单（repository inventory）精确取被审计
提交中 `git ls-files examples/data` 的全部 JSON，再由 formatVersion/Manifest binding 分类；
不能只列成功样例。

### 13.1 已发布资产范围

截至 2026-08-07 的 #297 产品事实是：项目尚未发布 1.0，也从未通过 GitHub Release、安装包
或直接用户交付三个正式渠道发布 current JSON 数据。v1 因而把已发布资产清单（published
inventory）冻结为 `status=complete` 的可验证空清单，而不是无法验证的 `not-applicable`
声明。范围 profile 固定为 `LF-CURRENT-PUBLISHED-INVENTORY-v1`，按顺序包含
`github-releases`、`installer-bundles`、`direct-user-delivery`，资产数固定为 0。

空清单摘要字节固定为 ASCII `LFCURRENT-PUBLISHED-INVENTORY-v1`、little-endian `u16`
渠道数、每个渠道的 little-endian `u16` 长度与 UTF-8 bytes，最后是值为 0 的 little-endian
`u64` 资产数；其 SHA-256 已知向量为
`5226af57e4d4d869f36b25b38b747f1e2d04820b93c3af11dff8dd00c3580ad3`。报告的 locator
固定指向被审计提交中的本节。若 G2/G3 前任一渠道首次发布 current JSON，必须回到 G1
提升已发布资产范围 profile 与报告 schema，不能继续复用空清单 pass。

repository inventory source 的 digest 是原样执行
`git ls-files -z -- 'examples/data/*.json'` 所得 NUL-separated bytes 的 SHA-256。v1 published
source 必须精确携带上述 scope profile、三个渠道、零资产数、固定 locator 与已知摘要；schema
禁止任何 published asset row。

`assets[]` 按 path 的 UTF-8 字节序唯一
排序。`inventoryDigest` 固定为 SHA-256(`LFCURRENT-ASSET-INVENTORY-v1` + 对每项依次编码
1-byte inventory kind、little-endian `u64 path_len`、path bytes、20-byte Git blob、little-endian
`u64 byteLength`、32-byte source SHA-256 raw bytes)。inventory kind 冻结为 repository=`0x00`、
published=`0x01`；Git/SHA-256 hex 必须先解码为 raw bytes。v1 没有 published row；后继 profile
即使增加它，也必须令 `gitBlob=null` 并在摘要中编码 20 个零 byte。G2 必须提供 canonical
encoder 已知向量测试。

逐项状态由 schema 的闭合分支裁决：配对迁移成功必须是 `paired-success + success + 空诊断`；
仓库内仅交通数据与非当前版本分别是 `unpaired-traffic-only` /
`unsupported-current-version + expected-failure + 非空诊断`；任何未处理迁移失败只能是
`migration-failure + unhandled-failure + 非空诊断`。所有报告都必须携带清理责任绑定；
`overallStatus=pass` 还要求前三个 expected=actual 分支以及从审计父提交独立验证的清理授权记录
全部精确匹配。存在任一 migration failure 时只能是 fail；fail 报告携带清理责任只用于归责，不能
授权退役。

清理责任记录固定使用 `laneflow.current-import-cleanup-authority` v1，并由
`current-asset-audit-v1.schema.json#/$defs/cleanupAuthorityRecord` 验证。字段按顺序为
`schema`、`schemaVersion`、`retirementProfile`、`sourceIssue`、`g2Evidence`、
`cleanupIssue`、`cleanupIssueNodeId`、`cleanupOwner`；其中 source Issue 固定为 #297，G2
evidence 必须是 #297 的规范 issue-comment permalink，cleanup Issue 必须是本仓库中区别于 #294
与 #297 的专用 Issue，owner 是不带 `@` 的规范 GitHub 登录名。精确 bytes 使用 UTF-8 无 BOM、
两空格缩进、LF 和单个末尾 LF。报告的 `cleanup.authority` 固定记录 profile
`LF-CURRENT-CLEANUP-AUTHORITY-v1`、上述固定路径和该文件 exact bytes 的 SHA-256，并重复可读的
Issue URL、Node ID 与 owner；这些重复字段必须由 validator 逐项比较，不能自证。

### 13.2 仓库证据提交与原子发布

报告审计干净父提交 A，`source.commit=A`；最终证据提交 E 的唯一父提交必须是 A，且 `A..E`
只能新增固定路径
`docs/reference/current-asset-audit-v1-<A>.json`。G3 exact head 是 E，G3 comment 同时记录 A、
E、固定路径与报告 Git blob；validator 必须实际验证父子关系和唯一文件差异，不能仅相信文件名。
rebase 或任何 A 内容变化都必须重新生成报告。

承载证据提交 E 的 PR 是仓库默认 Rebase and merge 的冻结例外：它必须使用
Create a merge commit 合入，使 A 与 E 的提交对象原样进入 `main`。该例外只适用于
`A..E` 仅新增固定报告路径的 PR，不扩展到该 PR 内的其他变更，也不要求任何其他 PR
改变合并方式。证据验证按合并前后拆分为两个阶段，各自冻结命令与失败条件。

合并前（G3，在 E 上执行，不依赖尚不存在的 merge commit）：

- `git rev-parse E^` 必须等于 A，`git diff --name-only A..E` 必须恰为固定报告路径；
- validator 按本节完整审计 A（清单重建、cleanup 责任、逐字段比对、负向矩阵）；
- 新鲜度闸口：`git merge-base --is-ancestor origin/main A` 必须成立，即证据分支与
  `main` 同步、被审计清单就是 `main` 当前清单；不成立时按 `main` 新父提交重新生成
  报告并重跑本节全部验证，不得以过期 A 合并。

合并后（G4/closure，在 merge commit M 上执行）：

- `git rev-parse M^2` 必须等于 E，A、E 可经 M 从 `main` 到达，标准 clone 可原样重放
  `A..E`；
- `git diff A M -- examples/data/ docs/reference/current-import-cleanup-authority-v1.json`
  必须为空，即合并后 `main` 的被审计清单与 cleanup 责任记录逐字节等于 A；M 不可达、
  `A..E` 不可重放或该 diff 非空都失败关闭，报告失效并须按 `main` 新父提交重新生成。

若该 PR 以 rebase、squash 或任何重写 A/E 提交身份的方式合入，报告中的 `source.commit`
与 G3 comment 记录的 SHA 立即失效，同样须按 `main` 上的新父提交重新生成报告并重新完成
验证。

Schema 只验证报告结构和闭合状态分支，`assets.minItems` 不证明其与动态仓库清单相等。validator
不得以报告内的 `assets`、repository source digest、`inventoryDigest` 或 `overallStatus` 作为枚举/
分类事实源；在验证 A/E 关系后，必须从 A 独立重建完整预期清单：使用全新的临时 Git index 对 A
执行 `git read-tree`，再原样执行 `git ls-files -z -- 'examples/data/*.json'`，以 stdout bytes 复核
repository source digest。不得从 E、调用方工作树或其普通 index 枚举。随后按该清单从 A 的 tree/
blob 读取每份精确 bytes，重新派生 Git blob、字节长度、source SHA-256、`formatVersion` 状态、
Manifest 配对、预期/实际分类、迁移结果与诊断；报告值不能参与这些派生。

同一 validator 还必须从 A 的 tree 固定路径读取清理责任记录，先验证 exact bytes 摘要和
`cleanupAuthorityRecord` 闭合结构，再把其中的 `cleanupIssue`、`cleanupIssueNodeId` 与
`cleanupOwner` 作为唯一预期值，与报告 `cleanup` 做逐字段精确比较。它不得从 E、工作树、报告内
authority path、调用方参数或验证时可变的 GitHub assignee 取得替代值；固定路径缺失、摘要或
profile 不匹配、G2 evidence 不是 #297 permalink、Issue 为 #294/#297、Node ID/owner 缺失或任一
报告字段不相等都失败关闭。

预期资产按 path UTF-8 字节序唯一排序后，validator 必须与 `assets[]` 做逐位置、逐字段、等长比较，
并从预期内容身份重新计算 `inventoryDigest`，从完整预期迁移结果重新计算 `overallStatus`。任何漏项、
额外项、重复 path、乱序、内容身份或分类/结果差异都失败关闭；v1 还须独立复核已发布资产固定空清单，
不能因为报告自洽而接受。validator 还必须确认报告 `profiles.compiler` 等于 A 上已登记、且生成本次
资产结果的实际编译调用所用配置档（v1 固定为 `LF-COMP-P100-INITIAL-v2`）；登记集合由 A 的编译器
配置档注册事实决定，其他已登记或未登记值都失败关闭。G2 负向测试至少逐项篡改遗漏、额外、重复、
乱序、Git blob、长度、
SHA-256、分类/迁移结果、repository source digest、`inventoryDigest`、`overallStatus` 与
`profiles.compiler`（含其他已登记值与未登记值），并证明
每类都被 validator 拒绝。清理责任负向矩阵还必须覆盖固定文件缺失、从 E/工作树替换、authority
profile/path/digest 篡改、非 #297 G2 evidence、#294/#297 冒充专用 Issue、Issue URL 与 Node ID
错配、owner 篡改，以及报告与 A 中责任记录任一字段不相等。

报告 bytes 固定为 UTF-8 无 BOM、两空格缩进、LF 换行、单个末尾 LF；object 字段按 schema
声明顺序，数组按本文规范顺序，由 importer 直接依赖的 `serde_json` 使用标准字符串转义。
精确 serializer 版本取 `source.commit` 对应 `Cargo.lock` 的解析结果，生成命令必须使用
`--locked`；G2 已知向量至少覆盖引号、反斜线、控制字符、ASCII、BMP 与非 BMP Unicode、空
数组/对象及单个末尾 LF，并逐字节冻结完整小报告，依赖更新造成的任何转义或排版变化必须先让
测试失败。报告必须由字段按 schema 顺序声明的有类型 `Serialize` 结构生成，不得先物化为
`serde_json::Value`/`Map` 再依赖 map 实现或 feature-unification 排序。工具在同目录写唯一临时
文件，完成 flush/file sync 后用同文件系统
`hard_link(temp, final)` 发布，从而取得 atomic no-clobber。`hard_link` 成功是唯一提交点：
此后报告已发布，不再允许以失败关闭回退；删除临时名或目录 sync 的失败只返回
published-with-cleanup-warning 状态并写入结构化诊断，残留临时名由下一次运行幂等清理，
不改变 `final` 内容与发布事实。`AlreadyExists` 时读取 final，只接受 byte-identical 幂等
结果；提交点之前的其他错误失败关闭。不得使用会覆盖目标的普通 rename，也不得在
hard-link 不可用时退化为非原子覆盖；提交点之前的任何失败都删除临时文件并保留既有报告。

当前冻结分类为：

| 仓库资产                                                      | 预期分类                                                                                     |
| ------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `v0.1-signalized-corridor.scenario.json` 及其 Traffic/Spatial | `paired-success`（配对迁移成功）；完整静态、pose、行为证据                                   |
| `v0.1-campus.scenario.json` 及其 Traffic/Spatial              | `paired-success`（配对迁移成功）；静态/空间与确定性证据                                      |
| `v0.10-parking-signals-baseline.laneflow.json`                | `unpaired-traffic-only`（仅交通数据、预期不迁移）；继续通过 Data 回归，不提交三文档 importer |
| `v0.10-multi-gate-waiting-zone.laneflow.json`                 | `unpaired-traffic-only`（仅交通数据、预期不迁移）；作为 current 回归基准，不得称格式无效     |
| 仓库 v0.2–v0.9 历史 Traffic                                   | `unsupported-current-version`（非当前版本、预期不迁移）；不可喂给 v0.10 importer             |

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
- strict 自定义测试迭代器覆盖：`len()` 过大但实际在 16 项内结束、`len()` 过小但实际仍在
  16 项内结束、克隆遍历与原遍历在上限内数量不同，均在相应遍历完成后返回输入契约诊断；
  `len()` 过大/过小但实际产生第 17 项，以及第二次遍历才出现超长 ref/第 17 项，均在对应
  增长前返回 source 资源诊断，不为等待契约比较继续遍历；
- 三种文档分别覆盖 `formatVersion` 缺失、显式 `null`、非字符串、重复 occurrence，以及
  unsupported version 与其他 DTO shape 同时存在的组合；前四类保持 `JsonShape`，重复时不选择
  任一 occurrence，只有唯一合法字符串版本才先于其他 shape 返回 unsupported version；
- Manifest 绑定前存在未引用的大 payload 时，该 payload 不进入 `selected_source_bytes`；source
  在 Manifest 哈希/换行索引/DTO 分配前先以 Manifest 实际长度检查 compiler source-byte 下界，
  再在绑定后以完整三文档值复核，且两次都覆盖边界、边界加一与失败前零规模分配；
- 每个资源维度执行边界、边界加一、先加其他模块、加入顺序变形、失败不污染和重试；
- materialized upfront charge 在读取来源文档前已入账：分别构造 `importer_build_id`/`provenance`
  复制与 source 边界输入之和恰好等于及超出调用点 `CompilerControlledLiveBytes` 剩余量的组合，
  前者成功、后者在任何来源分配增长前失败，且失败不保留部分模块、字符串或 backing；
- v1 compile profile 在读取/哈希/解析前以 `SourceDocumentCount` profile-incompatible
  失败，v2/后继显式多文档 profile 才能导入。

## 14. 性能收益、影响与实现代价

收益是消除 Traffic/Spatial/Manifest 的双 DTO、双版本判断和双配对权威，让资产迁移
直接复用唯一 compiler semantic pipeline，并把恶意或意外超大 current JSON 的风险限制
在 builder 的现有资源账本内。三个文档的来源身份和真实位置使差异、失败清单和后继
迁移审计不再依赖 current 数组下标与 LIR ordinal 偶然相同。

生产影响必须受控：`laneflow-data` 多一次 Manifest 文档摘要，但 Traffic/Spatial 摘要
仍各一次；production 不建立位置表或 strict lookup，不增加制品数/ref 拒绝条件。共享
泛型 DTO 会增加 source 包代码复杂度；两种策略共享的 record replay/token-local raw decode
会改变 production 与 strict 解析常数，strict 另承担换行索引和位置槽。它们不进入 Traffic
Runtime 固定步进，换来统一 version-before-shape 与真实定位。独立 source crate 还增加一个
unpublished 内部边界和迁移期维护成本；该成本
由阶段 8 后明确退役抵消，而不是形成永久兼容层。

source-owned 迭代器预检删除 compiler 侧一次规则镜像和最多 16 项 source view 的短 `Vec`
分配；它仍需对仅含借用元数据的克隆/原迭代器各扫描一次，并为严格入口产生一个泛型单态化实例。
相对 JSON 哈希、解析和 lowering，该常数成本可忽略且不进入 Traffic Runtime；收益是 profile
升级只修改 source 权威、compiler 无需同步数值或诊断。importer 的第三方支撑依赖全部只存在于
未发布离线工具闭包，当前 workspace lock 已包含这些包，因而 G2 预计不增加解析 crate；代价是
Cargo metadata 白名单与报告已知向量必须随每次依赖更新共同维护。

Manifest 字节下界检查在成功路径只增加两次整数余额比较，不扫描或复制输入；预算不足时反而避免
Manifest 哈希、换行索引和 DTO 分配。G3 validator 独立重建清单会对仓库 JSON 增加一次
`O(asset count + total bytes)` 的 Git tree/blob 读取、分类和 SHA-256，但只发生在未发布离线工具与
证据闸口，不进入 production loader、compiler 热路径或 Traffic Runtime。其代价是维护报告生成与
独立复核命令及负向篡改矩阵，收益是漏项或自洽伪造不能产生错误 `pass`。
从同一 A tree 读取清理责任记录只增加一次小型固定文件读取、Draft 2020-12 校验、SHA-256 和常数
字段比较；不会访问 GitHub 网络，也不随资产数增长。实际 importer build/provenance 只替换模块
描述符中原本就存在的两个字符串并增加一个 32 字节输入值，不增加 LIR 表、运行时镜像或固定步进
成本；代价是官方 importer 构建必须注入可审计构建身份，直接 compiler 宿主也必须显式登记自身
转换沿袭。

G2 基准显式继承 [`compiler-foundation.md`](compiler-foundation.md) 第 10.4 节：同一 P100
机器、相同 release 配置，每级至少 1 次预热和 7 次正式样本，报告 median、MAD、compiler
控制峰值与保留容量；任何可重复时延或内存回退必须定位并修复，或携带事实回到 G1。
current 专用报告分别覆盖：production-compatible 场景加载基线/候选及其 record-token replay
字节与时延；strict Manifest、Traffic、Spatial 解码及其 replay；SHA-256/换行索引；位置冻结；current
lowering；共同 admission；完整 compile。不得把 strict 解析成本归入 #315 admission
regression，也不得只报告成功结果 live bytes。

每份原始文档的根 deserializer 和 SHA-256 调用次数必须由测试计数器证明为 1；两种策略都
证明每个固定 schema record token 至多 replay 一次。production 不建立位置表，成功路径和
错误路径都不得执行第二次根解析；与现有双阶段 loader 相比出现稳定 production 回退时必须
修复或回到 G1。

## 15. G2 实施切片

只有 #297 已取得 G1 Pass，并基于当时 exact `main` 完成 GitHub 元数据、依赖关系、#315
`G4 Exception` 继承边界和 cleanup Issue/owner 复核且追加独立 `G2 Pass` 后，才按以下顺序
落地；任一切片都不能提前写入 Runtime：

1. 按 G2 append-only 授权记录提交固定路径清理责任文件，并以 A-tree 读取、摘要和字段错配负向
   测试冻结其独立验证边界。
2. 新建 `laneflow-current-source`，迁移 wire DTO、版本/摘要/配对与 Traffic-only 能力；
   用兼容矩阵证明 `laneflow-data` accepted set 未收窄后删除 Data 的重复 DTO/配对实现。
3. 增加共享增长前 visitor、有界 record-token replay、static location policy，以及 strict
   profile、packed 位置槽、换行索引、位置闭合集合和 source 自身边界测试。
4. 在 compiler 默认关闭特性下增加 `CurrentSourceArtifact`/`CurrentImportProvenance`/
   `CurrentSourceInput`、余额派生、current 私有降阶、三文档描述符/源映射和原子 admission；补
   compile-fail/API/DAG 测试，并证明不同 build/options/provenance 原样进入只读模块描述符。
5. 新建以 compiler 为唯一 LaneFlow 直接依赖的薄 `laneflow-current-import`，并使用第 3 节
   冻结的第三方支撑依赖白名单，实现批量调用、符合 `current-asset-audit-v1.schema.json` 的
   机器可读资产报告、失败清单和从 A 独立重建完整清单的 validator；文件读取、路径解析和原子
   输出只在该宿主工具层。
6. 完成 `LF-COMP-CURRENT-EQUIV-v2`、release 性能/内存、全 workspace CI、外部审阅、G3/G4
   证据；#294 只有在 paired success 与显式失败清单完整后才能消费删除前置。

## 16. G1 冻结清单

- [x] crate DAG、第三方支撑依赖白名单、锁文件唯一解析权威、默认关闭特性、能力可见性和
      退役条件闭合；
- [x] `CurrentSourceArtifact::new`、`CurrentImportProvenance::new`、
      `CurrentSourceInput::new` 与唯一 builder 入口精确；
- [x] production-compatible / strict / Traffic-only 三条能力、source-owned 迭代器预检及
      不可转换边界精确；
- [x] 一个逻辑模块、固定 namespace、三文档键、来源语言和逐文档 origin 精确；
- [x] 单一 DTO/摘要/配对权威、静态位置策略、额外制品语义和失败顺序精确；
- [x] `LF-CURRENT-SOURCE-P100-IMPORT-v1` 全部固定值、builder 余额派生和 live 事务精确；
- [x] Manifest/Traffic/Spatial 字段与记录位置闭合集合精确；
- [x] current external ID、迁移派生 key、owner/coverage/geometry 降阶精确；
- [x] 仓库资产分类、已发布资产空清单、A-tree 清理责任事实源、证据提交、
      等价/确定性/资源/兼容/性能矩阵精确；
- [x] 本地全面审阅、exact-head `ae5f0898` 外部 clean review 与 #297
      [`G1 Pass`](https://github.com/illusion-tech/laneflow/issues/297#issuecomment-5222064282)
      已完成。

本文件处于 Accepted；#297 G1 Pass 只授权本收口，不授权开工。实现仍须按第 15 节
基于当时 exact `main` 完成复核并另行取得 `G2 Pass`。
