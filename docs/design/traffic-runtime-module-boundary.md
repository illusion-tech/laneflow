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

内核不导入 `laneflow_format` 或 `laneflow_runtime_snapshot_wire`，不引用管理平面的
快照、恢复、摘要、切换或格式准入模块。内核只保留两项必要的管理连接：安装时构造
`AdministrativeState`，以及按阶段协议向迁移日志写入已提交变化。迁移日志是有界的
进程内增量记录，不能因此成为 LFSD/LFRS 解析入口。日志溢出仍只使候选切换失败，
旧世界继续步进；本拍失败仍不追加日志。

观测与 Routing 会话继续由各自对象持有导出/接入状态；切换候选由
`CutoverTransaction` 持有。不能为了目录整齐把它们并入活动世界。

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

这里的准入结果可以是完整的局部新世界，不要求增加持久化中间格式、公共能力 token
或全量 DTO。物理拆 crate 和公开 API 的重新组织由独立任务决定。

## 3. 架构检查

运行：

```powershell
cargo +1.98.0 run --locked -p xtask -- check-runtime-architecture
```

检查器从 Runtime `lib.rs` 遍历 Rust 模块树，解析 import、别名、re-export、类型/表达式
路径和宏中的路径；格式入口同时按逻辑模块和物理文件定位。它禁止内核引用上述管理
操作，也禁止其他生产模块直接使用格式依赖。解析失败、必需模块缺失、替代模块路径、
动态 `include!` 或无法证明边界的通配导入都会返回失败。

只有能够证明不进入非 test 构建的 `cfg(test)` 项被排除。条件 feature、平台及未知
cfg 均保守纳入；`any(test, feature = "...")` 不算测试专用代码。测试夹具可以通过
格式/编译器构建受检输入，不能把这种依赖带入生产分支。

Cargo metadata 中的生产与 build 依赖（含 optional、重命名和按目标配置的声明）用于
检查传递方向：Runtime 与 Spatial 互不依赖，两者都不能反向到 compiler、Bevy Adapter
或 Bevy 引擎。Adapter 可以依赖 Runtime 与 Spatial。dev 依赖单独排除，避免把既有
集成夹具当作生产依赖。

该检查加入 PR、Merge Queue 与 main 共用的 `Rust checks`，与现行 wire 审计共用
已构建的 xtask 二进制；违规案例测试验证命令的拒绝路径。模块依赖检查与 Rust 编译、
阶段借用权限和行为测试共同构成证据，不改变现有五项 required check 名称。

## 4. 验证边界

复用阶段等价轨迹、首错、故障重试、快照回放、切换/增量追赶、生命周期、保留内存
账本和暖机后分配证据。文件搬迁只更新 import 与夹具的相对位置，不更新对照轨迹。
源码重组不改变交通算法、工作线程数、共享静态权威或任何 wire 版本。
