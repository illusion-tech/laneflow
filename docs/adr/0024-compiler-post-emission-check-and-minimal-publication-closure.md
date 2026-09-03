# ADR 0024：编译器后发射检查与最小发布闭合

**状态**: Accepted<br>
**日期**: 2026-08-18<br>
**适用范围**: LFCA/LFSM/LFSD 最终字节检查、`laneflow-format` 职责、LFCP、
compiler 制品边界、宿主发布分工、#300/#302 的上游输入边界<br>
**部分取代**: ADR 0020 中独立 `laneflow-validator`、规范发布验证收据、
`canonical-publication-v1` receipt 及由 #299 统一交付三类 receipt 的决定；同时取代
ADR 0021 对独立 validator/receipt 的依赖假设；不改变编译器拥有静态路网、Canonical
LIR、可移植规范制品、目标静态镜像或对象外信任锚决定<br>

> **后继决策（2026-08-18）**：ADR 0025（Accepted；#300 G1 Pass）不改变本文
> LFCA/LFSM/LFSD、LFCP v2 或后发射检查；它把本文面向 #300 的“构造目标静态镜像”
> 下游改为“从同一受检 LFCA capability 构建进程内 `SharedNetworkRevision`”，并取消
> 独立镜像发布对象。
>
> LFCA / LFSM / LFSD 的分块容量合同把本文输入从“三份完整 slice 同时驻留”修订为
> 三个由 `laneflow-format` 封闭构造、保证没有 LaneFlow safe API 可达写能力的不可变、有界、可重读对象
> 来源。完整 slice 只是零复制 adapter；候选、检查能力与共享静态构建可以保存或借用冻结
> source handle，不要求 `Box<[u8]>`。LaneFlow 不拥有内容仓库、原子文件安装或 manifest
> 提交；这些由选择持久化制品的宿主负责。

**关联文档**:

- `0020-compiler-owned-static-network-and-static-image.md`
- `0025-checked-canonical-network-and-shared-static-network.md`
- `../design/shared-static-network.md`
- `../design/compiler-post-emission-check-and-minimal-publication-closure.md`
- `../design/network-compiler.md`
- `../design/portable-canonical-artifact.md`

## 背景

ADR 0020 原计划让独立 `laneflow-validator` 不复用 compiler 语义实现，重新计算身份、
所有权、拓扑、几何、规则、路网修订和三类验证收据。#298 随后交付了
`laneflow-format`、LFCA/LFSM/LFSD、LFCP v1 与一套内容存储原型，
并为未来 receipt 保留了接口与线格式槽位。

在 pre-1.0 阶段继续建设第二套完整语义实现，会同时增加算法漂移、crate/API 冻结、
验证线格式和长期兼容负担。即使独立 validator 与 compiler 共享同一语义后端，它也不能
发现二者共同存在的系统性缺陷；此时保留独立产品和 receipt 只增加形式分层，不增加相应
信任价值。

当前更优先的产品目标是让完整编译、制品交付、共享静态路网和 Traffic Runtime 链路
落地，并控制性能、架构复杂度和维护成本。#299 因而收缩为对最终字节执行不可绕过的
后发射检查，并向宿主提供 LFCP binding；不把尚无产品调用者的内容仓库事务设为核心前置。

## 决策

### 1. 不交付独立 validator 或第二套语义实现

#299 不创建 `laneflow-validator` crate、独立可执行程序、验证服务或通用证明平台，
也不重新实现 compiler 已拥有的身份、所有权、拓扑、几何、规则和差异语义。

compiler 继续是来源、IR 和静态路网语义的唯一编译权威。其单一语义实现的系统性缺陷
风险由人工可复核固定向量、compiler 测试、真实场景回归和历史缺陷断言控制，不通过
复制完整生产语义来控制。

### 2. 共享后端扩展现有 `laneflow-format`

`laneflow-format` 在既有结构、登记和值域预检之上增加 bundle 级后发射检查。
`laneflow-format` 继续依赖 `laneflow-static-contract`；compiler 和后继消费者依赖
`laneflow-format`。不新增中间检查 crate，也不让 #300 为消费受检视图而反向依赖
整个 compiler。

依赖方向固定为：

```text
laneflow-compiler ──┐
                    ├──> laneflow-format ──> laneflow-static-contract
#300 static-image ──┘
```

checker core 继续不依赖 `std`、文件系统或内部并行。百万级路径允许
`laneflow-format` 提供可选 `std` closed-staged backing adapter；它只把平台不可变 handle
实现为 sealed source，不进入格式解析、hash/binding 算法或通用 installer。若 checker core
本身以后需要文件系统/并行，或出现第二种独立规范制品生产者，仍须重新进入 ADR。

### 3. 检查最终字节与跨对象绑定，不复验完整路网语义

公共后发射入口只接受：

- 最终关闭、exact length 已知、由 `laneflow-format` sealed capability 证明 backing 在检查和
  后续能力消费期间没有 LaneFlow 可写能力的 LFCA/LFSM/LFSD 可重读对象来源；
- 显式 `ExpectedSemanticDiffBase`；
- 调用方 `FormatLimits`。

它负责：

- 既有结构、登记和直接值域预检；
- 三个对象的摘要和精确长度；
- 从 LFCA exact bytes 重算 `NetworkRevisionId` 并比较声明值；
- LFSM 到 LFCA 的修订、摘要和长度绑定；
- LFSD target 到 LFCA、LFSD base 到显式 expected base 的绑定；
- 每对象与实际 staged/resident 资源限制；不得把旧 chunk scratch 常量当作完整 bundle 上限。

它不负责：

- 从来源、AST、HIR、MIR 或 LIR 重建预期制品；
- 逐实体重新执行 BLAKE3 身份派生或建立第二份碰撞登记表；
- 重新裁决完整所有权、拓扑、几何或规则语义；
- 重新生成 LFSD 并证明差异完整无遗漏；
- 证明来源、发布者或 manifest 的真实性；
- 证明运行时迁移安全。

### 4. 使用 owning capability 守卫 LFCP 与共享静态构建

`laneflow-format` 提供字段私有、无公共构造器的
`PostEmissionCheckedBundle<L, M, D>`。safe downstream 不能为普通文件路径、可写映射、
内部可变 buffer 或 callback 实现来源 trait；immutable slice/owned bytes 通过 safe 入口。
百万级文件路径由字段私有 staged writer 在 flush、固定 file identity/exact length 并结束
writer 写阶段后，构造 sealed `ClosedStagedObjectSource`。能力内部可以继续持有具写访问权的
字段私有 `File`，但 finish 后没有 LaneFlow safe API 可达的写能力；safe API 不暴露路径、
原始文件或重新取得写能力的 token。checker 与共享静态构建直接复用同一 immutable
capability/backing。identity、length 或 bytes 漂移时失败
关闭。该能力不可序列化，不表示对象已经持久化、发布、认证或可信。完整 slice 通过同一接口
的零复制 adapter 进入，不建立第二个 checker。

compiler 使用拥有不可变 staged source handle 与 expected base binding 的
`PortablePublicationCandidate`；候选可以由完整 slice 支撑，但百万级路径不要求三份
`Box<[u8]>` 同时驻留。LFCP v2 builder 与共享静态 builder 只能消费该局部受检状态。

### 5. LFCP v2 一次性移除 receipt

LFCP v2 只保存：

1. LFCA 的版本、路网修订、摘要和精确长度；
2. LFSM 的版本、摘要、精确长度与 compiler/source provenance；
3. publisher provenance 及 LFCA/LFSM 内容寻址对象键。

LFCP v2 不保存 receipt、LFSD、检查清单、证明版本、策略、签名或认证字段。LFSD 接受最终
字节检查，但 LFCP 对它没有发布或切换语义。

LFCP v1 和 `CanonicalPublicationReceiptViewV1` 退出生产实现。不提供 v1/v2 双版本
读取，不把 v1 字段静默解释为 v2。历史固定向量和 #298 证据保留其原始含义，并明确由
本 ADR 取代。

### 6. 宿主拥有持久化与认证

本决定不引入新的证明制品。LFCP v2 是内容寻址发布描述符，不自行证明已经持久化或发布；
`PostEmissionCheckedBundle` 是进程内检查能力，也不自行证明真实性。宿主、CI、打包工具或
发布服务决定如何落盘、并发协调、崩溃恢复、签名与提交 manifest。对象外的认证 manifest、
宿主认证包或 pinned digest 继续是真实性来源；加载方必须重新验证宿主交付的 exact bytes。

若未来 LaneFlow 自建并发内容寻址仓库，必须单开 ADR/Issue；不得把 atomic no-replace、
winner、目录 fsync 或 Windows/Unix 文件事务重新并入 compiler/format/Runtime 核心合同。

### 7. SHA-256 由 `laneflow-format` 直接计算

`laneflow-format` 使用既有 `sha2 0.11` 且关闭默认 features，自行计算对象摘要和
`NetworkRevisionId`。不接受调用方 hash callback 或可替换算法 trait。

这不会引入新的第三方包；compiler 可继续直接使用同一依赖计算 compiler-private
来源集合摘要。

### 8. 资源和性能保持有界

后发射检查必须：

- checker core 保持 `no_std`；可选 `std` staged adapter 不改变检查算法；
- 不发生堆分配；
- 不复制 LFCA/LFSM/LFSD；
- 按三个来源的总 exact bytes 线性扫描，不复制或把完整对象物化为 heap buffer；
- 不创建线程或内部并行任务；
- 在解析或 hash 前用 O(1) 长度检查拒绝超限输入；
- 每个候选在 LFCP 构造或共享静态构建前执行一次完整 bundle 检查。

资源证据在单线程 release 配置下覆盖 `10000` / `100000` / `1000000` 个现实混合稳定静态实体，
逐阶段记录 source build、compile、file-backed emit、checker 与共享静态构建的墙钟和内存。
代码与类型路径审计证明 staged 路径不构造完整对象 heap buffer；每档至少执行一次完整端到端
路径，证明百万级路径可达、返回候选不常驻完整 heap bytes、阶段峰值有记录且 checker 零 heap
allocation。单次资源样本不独立证明 emit 过程从未短暂分配完整 buffer，也不是统计性能结论或
Product Pass；无需新增 benchmark crate、JSON 协议或常驻性能平台。

### 9. 共享构建与切换保持独立信任边界

本 ADR 只决定 canonical artifact 与宿主 publication 的边界。共享静态构建和跨修订
切换由各自权威合同负责；它们不得假设 compiler 会交付独立 validator 或
`static-image-v1`/`revision-cutover-v1` receipt。

LFSD 通过 #299 后发射检查只说明最终差异对象的直接值域和 base/target binding 闭合，
不构成目标修订的激活决定、迁移可行性证明或切换提交点。

## 后果

正面后果：

- 只维护一套生产语义实现；
- #299 成为小而可测的制品硬化切片；
- #300 可以复用中立格式能力而不依赖 compiler；
- 删除未实现 receipt 的 wire、安装步骤和错误面；
- 性能成本可直接与既有 P100 emitter 基线比较。

代价与接受风险：

- 后发射检查不能发现 compiler 与共享格式契约共同存在、且不破坏字节闭合约束的系统性
  语义缺陷；
- LFCP v1 内部固定向量发生一次明确的不兼容替换；
- #300/#302 必须重新打开旧 receipt 假设；
- `laneflow-format` 的职责从单对象格式预检扩展到小范围跨对象制品闭合。
- 持久化宿主必须自行承担内容仓库事务与认证提交，LaneFlow 不提供通用 installer。

## 被拒绝的替代方案

### 保留独立 `laneflow-validator`

当前需要复制完整语义实现，维护成本与实际 pre-1.0 风险不成比例。

### 新建 `laneflow-artifact-check`

它可以形成更纯粹的分层，但当前检查规模和依赖不足以证明新增 crate/API 的长期价值。

### 把检查全部留在 compiler

这会迫使 #300 反向依赖 compiler，或只能信任 compiler 自报的结果。

### 在 compiler 内建内容寻址仓库

玩家道路编辑与当局编译不需要文件安装；当前也没有内容仓库产品调用者。把 atomic
no-replace、并发 winner、目录耐久和平台文件事务作为后发射前置，会扩大核心实现面却不改善
玩家热路径。真正需要仓库时应由宿主实现，或另开 LaneFlow 存储设计。

### 保留 receipt 并由 compiler 签发

同一进程、同一共享后端生成的 receipt 不增加独立信任，只重复绑定并扩大线格式和安装
事务。

### 原地修改 LFCP v1

这会让相同版本号表达不同结构和语义，破坏 #298 固定向量与版本纪律。

### 在本 ADR 同时决定静态镜像和修订切换证明

这会重新扩大 #299，并越过 #300/#302 的产品与实现决策。
