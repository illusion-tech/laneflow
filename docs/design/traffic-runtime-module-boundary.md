# Traffic Runtime 仿真与管理模块边界

**状态**: Active<br>
**适用范围**: `laneflow-runtime` 私有模块、格式入口与生产依赖方向<br>
**关联合同**: [串行阶段协议](traffic-runtime-phase-protocol.md)、
[共享静态消费](traffic-runtime-shared-consumption.md)、
[运行时快照](traffic-runtime-snapshot.md)、
[修订切换](traffic-runtime-revision-cutover.md)

## 1. 模块与所有权

保持一个 crate 和一个公开 `TrafficWorld`。私有源码组织为：

```text
src/
  kernel/  状态、运行方法、固定步进、车辆/路线、占用与交通规则
  admin/   快照捕获/恢复、修订切换、迁移日志及格式入口
  facade/  唯一世界聚合、来源值、观测与 Routing 宿主会话
  lib.rs   现行公开类型与函数的 re-export
```

`facade::TrafficWorld` 组合世界绑定、已提交状态、派生索引、工作区与管理状态；
`kernel/world.rs` 保留安装及运行方法。管理状态的所有者位于 `admin/state.rs`，
日志武装、解除与在途事务放弃方法位于 `admin/migration_journal.rs`。
目录划分不改变公开入口、字段所有权、实例身份或借用寿命，也不新增公共模块或 prelude。

内核生产源码的显式路径不引用 `laneflow_format`、`laneflow_runtime_snapshot_wire`
或管理平面的快照、恢复、摘要、切换、格式准入模块。内核保留两项必要的管理连接：安装时构造
`AdministrativeState`，以及按阶段协议向迁移日志写入已提交变化。迁移日志是有界的
进程内增量记录，不能因此成为 LFSD/LFRS 解析入口。日志溢出仍只使候选切换失败，
旧世界继续步进；本拍失败仍不追加日志。

观测与 Routing 会话继续由各自对象持有导出/接入状态；切换候选由
`CutoverTransaction` 持有。不能为了目录整齐把它们并入活动世界。

本边界约束源码归属、格式入口和包依赖。`TrafficWorld` 仍是同一 crate 内的公开
聚合类型，分散在不同目录的 inherent impl 不构成方法调用权限隔离。具体阶段计算
继续由阶段协议的受限视图约束；完整的 kernel 到管理操作调用隔离需要另行设计
内部能力接口，不能由目录位置或本架构检查命令推断已经实现。

## 2. 唯一格式入口

`admin/format_admission.rs` 是 Runtime 生产源码中唯一直接使用格式读取器和快照
生成绑定的文件：

- LFSD 依次验证精确长度、字节摘要、登记结构和值域、base/target 制品绑定。
  切换描述符仍先执行原有 O(1) 上限预检；认证不授予新的迁移策略。
- LFRS 保持 verifier-first、版本/绑定/容量检查、稳定引用 lowering 和原子新世界
  恢复的原有先后。lowering 直接构造尚未发布的局部 staging world，通过既有运行时
  入口验证交通不变量；不先物化另一份全量解码快照，不重复编译路线。
- 编码也集中在同一文件，只消费 `CapturedSnapshot`，不回读活动世界。

`snapshot.rs` 拥有捕获逻辑与编码无关的快照值；`snapshot_restore.rs` 拥有公开上限、
错误、恢复结果及入口。它们调用格式入口，格式入口不向其他模块返回原始 wire view。
版本、首错、未知字段拒绝、错误值、逻辑摘要、失败原子性及原有分配预算保持原合同。

格式入口的生产模块接口限定为以下三个 `pub(super)` 函数，参数与返回值采用现有
具体类型；入口内部的私有解码辅助函数可以使用 wire 类型：

| 函数                   | 输入                                           | 输出                                             |
| ---------------------- | ---------------------------------------------- | ------------------------------------------------ |
| `verify_semantic_diff` | 可选语义差异绑定、LFSD 字节及 base/target 来源 | `Result<(), CutoverDescriptorError>`             |
| `encode_lfrs`          | `&CapturedSnapshot`                            | `Vec<u8>`                                        |
| `restore_lfrs`         | LFRS 字节、共享修订、来源、世界配置与恢复上限  | `Result<RestoredSnapshot, SnapshotRestoreError>` |

该模块不另行提供可见类型、trait、关联项、常量或 re-export，也不通过泛型、
`impl Trait` 或 trait object 扩大这三个函数的接口。新增出口应先修改本合同及其
检查证据。这个有限接口约束避免让检查器承担任意 Rust 类型的数据流分析。

这里的准入结果可以是完整的局部新世界，不要求增加持久化中间格式、公共能力 token
或全量 DTO。物理拆 crate 和公开 API 的重新组织由独立任务决定。

## 3. 架构检查

运行：

```powershell
cargo +1.98.0 run --locked -p xtask -- check-runtime-architecture
```

架构检查的交付合同由下面三组规则组成。检查器分别取得 Cargo 依赖图、生产模块
清单和格式入口声明，再执行对应规则；不将语法路径当成已经解析的方法调用目标。

### 3.1 生产模块与显式路径

- 业务模块采用显式 `mod` 声明组织。从 Cargo 声明的实际 Runtime 库入口递归建立
  这些声明对应的完整生产源码清单；宏展开生成的模块不属于已验证清单，采用此类
  组织形式须先扩展合同和检查方式。crate 根仅承担模块声明与既有 re-export；
  业务实现归入 `kernel/`、`admin/`、`facade/`，不允许另增
  `legacy` 等根模块规避分区规则。逻辑模块及物理文件须落在对应目录内。
- 检查显式 import、别名、re-export、类型/表达式路径及源码宏 token 中的路径。
  除唯一 `admin/format_admission.rs` 外，生产模块不得显式引用格式依赖；kernel
  的显式路径也不得指向 §1 所列管理操作模块。对入口同时核对逻辑名和物理文件。
- 按 §2 核对格式入口的三个具体函数及可见声明，不能只检查部分语法节点后推断
  所有出口安全。对额外可见 trait 等声明直接拒绝，不为其实现通用类型解析。

只有能够证明不进入非 test 构建的 `cfg(test)` 项被排除。条件 feature、平台及未知
cfg 均保守纳入；`any(test, feature = "...")` 不算测试专用代码。测试夹具可以通过
格式/编译器构建受检输入，不能把这种依赖带入生产分支。

### 3.2 包依赖方向

Runtime 与 Spatial 互不依赖，两者都不能反向到 compiler、Bevy Adapter 或 Bevy
引擎；Adapter 可以依赖 Runtime 与 Spatial。

- 本仓库的 normal/build 依赖声明全部纳入，包含 optional、重命名和按目标配置的
  声明。dev 依赖不作为生产边遍历。
- 外部包沿 Cargo 完整解析图的 normal/build 边继续遍历，以 Package ID 区分节点，
  不能因包不在 workspace 内便终止，也不能仅按包名合并不同版本。
- 解析配置覆盖工作区默认特性和 `--all-features`，不使用平台过滤；原有锁文件必须
  保持不变。外部包未在这些配置中启用的可选依赖不属于已解析的生产图，不递归打开
  所有第三方特性组合。
- 未取得必需解析结果或遇到应已解析却缺失的节点时失败；不把不完整图报告为通过。

### 3.3 失败含义与能力上限

“失败关闭”适用于上述规则的输入与判定：解析失败、入口或必需模块缺失、目录逃逸、
不支持的模块 `#[path]` / `cfg_attr`、动态 `include!`、通配导入或依赖图缺失都会
返回失败。未知的输入结构不能被静默跳过后报告相应规则通过。

该命令预防日常开发中的结构和依赖回退，不是通用 Rust 语义分析器。它不执行类型
推断、trait 方法解析、宏展开或任意值的数据流追踪，也不证明
`world.capture_snapshot()` 与 `TrafficWorld::capture_snapshot(world)` 这类调用
不可达。方法名黑名单不能替代调用隔离。修改检查器、CI 或依赖实现以刻意绕过约束
的贡献者，也不属于此命令提供安全隔离的对象。

检查通过只表示这些明确列出的规则通过。该范围不免除显式格式依赖漏检、可见接口
越界、缺失生产模块或已解析依赖漏查等缺陷的修复责任。

该检查加入 PR、Merge Queue 与 main 共用的 `Rust checks`，与现行 wire 审计共用
已构建的 xtask 二进制。模块依赖检查与 Rust 编译、阶段借用权限和行为测试共同
构成证据，不改变现有五项 required check 名称。

## 4. 验证边界

复用阶段等价轨迹、首错、故障重试、快照回放、切换/增量追赶、生命周期、保留内存
账本和暖机后分配证据。文件搬迁只更新 import 与夹具的相对位置，不更新对照轨迹。
源码重组不改变交通算法、工作线程数、共享静态权威或任何 wire 版本。

检查器验收按规则选取有限的正反例：合法当前树、合法 test/dev 夹具、额外根模块、
直接及别名格式引用、额外可见 wire/trait 出口、外部传递违规边和不完整输入。
源码用例须说明是否可编译；拒绝测试不能仅依赖本来就不合法的 Rust。需要证明
Cargo 图采集的规则，应包含实际 metadata 采集用例，不能只有手写图单元测试。

新增审阅反例先归属到本合同的具体规则。属于规则内的漏检必须修复；要求完整调用
图、通用宏展开或第三方全特性组合的扩展，应先单独定义收益、成本和验收范围。
不得把尚未交付的调用级能力隔离记为本次已经完成。
