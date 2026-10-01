# 交通运行时列式批量执行

**文档状态**: Accepted<br>
**设计入口**: [#814](https://github.com/illusion-tech/laneflow/issues/814)<br>
**关联决策**: [ADR 0031](../adr/0031-columnar-motion-authority.md)<br>
**既有合同**: [阶段协议](traffic-runtime-phase-protocol.md)、[精确并行](traffic-runtime-parallel-execution.md)、[整数几何](traffic-runtime-integer-geometry.md)

本文冻结维护者授权的目标架构和实施边界，不声明已实现或已测得收益。当前车辆特化仍采用整数毫米权威；其它交通执行域未实现。实施进度、测量、未通过项留在 #814 和交付 PR。

## 1. 权威与三种位置

活动运动池（Active Motion Pool）使用分块列式布局（Block SoA）。初始块容量为 128 行；256 行仅作为后续实验参数。每块的热字段是路线游标、毫米进度、毫米每秒速度、微米 carry 和有效位集。身份、路线/profile/class/车长为稳定上下文；Waiting membership 和 maneuver traversal 属于稀疏控制状态。非活动记录保留在独立存储中，不进入活动热块扫描。

稳定上下文按逐车共同消费者组织为紧凑记录，减少组装逻辑值时的分散读取和重复边界检查；它不包含运动数值，不随 Current/Next 交换。只需要状态或身份的消费者直接读取受检目录，不为这些查询物化完整 `VehicleState`。

热消费者使用同一 attempt 内的只读行绑定，先验证完整句柄、generation 和物理位置，
再按需读取 Current/Next 的位置及控制字段；借用存续期间不能迁移目录或发布运动列。
P2 筛选不组装整车，实际预览才取得完整逻辑值。资源收尾的 Gate/Waiting 遍历直接消费
位置、车型和成员字段；完整 passage/eligibility 检查仍可以按需组装复杂行。
非入口 Gate 筛选保留同 occurrence 的边尾情况；事件减量必须同时排除 Waiting 计划、
旧成员、Clearing、grant 回看、reservation 及 passage 暂存，不能仅凭游标未变跳过。

车辆目录（Vehicle Directory）按完整句柄索引保存 generation、存储类别和物理位置。完整句柄决定身份，规范逻辑位置（Canonical Logical Position）决定消费/事件/首错顺序，物理行（Physical Row）决定连续读写及独占执行。三者不可混用。不按车道或 profile 每拍全局重排权威列。

`VehicleState` 是按值组装的逻辑视图、公开读取、快照及测试类型。内部返回 `&VehicleState` 的接口改为值或受限列视图；不维护完整 AoS 影子。登记/恢复/切换可以一次性拆分逻辑值；步进不得依靠全量 AoS→SoA→AoS 转换维持运行。

Active 进入/退出、销毁和替换由准备完毕的生命周期转移更新目录；孔洞只通过有效位跳过。压紧发生在借用已经归还的安全边界，必须同时更新完整句柄映射；失败 attempt 不迁移已提交行。

## 2. 同拍共享输入

绑定期只保存已受检静态/profile 表，不能让 Runtime 获得编译器或共享路网的第二份权威。P2/P5 共用同一 C(T)、完整车辆句柄和 attempt 上的基础查询：路线/context、当前限速、horizon、精确 leader gap、route end、signal/parking stop，以及真实需要的未量化提案。

只对现有消费者要求的车辆生产基础数据，P2 筛掉的预览不得因批量化重新全算。字段组按消费者生命周期保存，块级有效位与 touched 集合负责失效，不为每字段维护逐车 epoch。基础列每 Active 的初始预算为 128 字节；超过预算必须说明消费者和测量依据，不让各阶段分别复制同一份。

现有 `iidm_step` 只依赖速度、期望速度、leader gap、profile 和步长；movement stop 在 `si_comfort_travel` 的后续投影生效。P4 新增 Waiting/Conflict stop 时，相同提案输入可以复用未量化 IIDM 结果，但必须重新执行安全限制、积分、量化、carry、路线行走与到达。其它提案输入改变时重算 IIDM；不能只截短旧位置。现有完整预览 `reuse` 证明成立时可直接交付其结果。

跨屏障仅物化必要列；同块、同消费者、无外部依赖的步骤尽量融合。工作掩码区分 needs_preview、has_base_proposal、full_preview_reusable、needs_final_projection、needs_route_walk、has_resource_obligation 和 has_error。

## 3. 内核和 ISA

独立 `laneflow-motion-kernel` crate 仅拥有纯数值执行，接收长度验证一致的只读/独占可写切片；不接收车辆、路线、资源权威或回调。Runtime 继续继承工作区 `unsafe_code = "forbid"`。该 crate 单独允许必要的 unsafe，并 deny `unsafe_op_in_unsafe_fn`；所有 ISA 调用、指针 load/store、尾部与别名证明集中在该边界，禁止扩散到 Runtime。

提供同布局标量、AVX2、AVX-512 后端；初始化执行资源时检测 CPU/OS 支持并选择整块内核。强制后端只用于受检研究与差异测试，不写入交通身份、快照或摘要。每种 ISA 的完整循环位于同一 `target_feature` 边界；尾块不越界，不因单个复杂通道把整块退回标量。

前方存在限速下降不必然要求分段求值。使用相同 f32 顺序计算 IIDM 提案速度和完整制动约束窗的保守上界；只有块内所有行都能证明下降点在约束窗外，才采用融合数值流程。窗内、边界相等或材料不齐仍保留完整限速和位移约束，不用直接可行性判断替换原求解器。

完整纵向流水线覆盖整数→SI 转换、IIDM、停止点选择、安全速度限制、硬空间、f64 ties-even 量化、carry 和不跨边推进。静态降速边界、跨边权限、停车/Waiting/Conflict 等控制输入仍须精确准备；复杂通道使用精确路线查询和掩码多 hop 行走。没有“本拍最多跨一边”的假设。

保持每车表达式顺序，禁止 FMA、fast-math、近似倒数和半精度。`u32` 不截为 `i32`；跨 hop gap 为 `i64`；瞬时 SI 为 `f32`；受检量化为 `f64`。长路线保留分段坐标和 `BoundedDistance`，Finite/BeyondFinite/unknown/absent 不混为一个 bool。后端不改变舍入模式，不要求跨平台位级承诺。

AMD 官方规格支持 9955HX 的 AVX2/AVX512；实际进程可用性仍以运行时检测为准。Rust 1.89 已稳定相关 AVX-512 target features/intrinsics，使用仓库 Rust 1.98。资料：[AMD](https://www.amd.com/en/products/processors/laptop/ryzen/9000-series/amd-ryzen-9-9955hx.html)、[Rust 发布说明](https://blog.rust-lang.org/2025/08/07/Rust-1.89.0/)、[ISPC 数据布局指南](https://ispc.github.io/perfguide.html#use-structure-of-arrays-layout-when-possible)。资料支持可行性，不替代本项目性能证据。

## 4. 独占输出、首错与提交

任务按物理块获得独占 Next 列；规范 rank→row 的映射在整个 attempt 固定。输出有效位必须覆盖所有被消费 Active 行，包含完整预览复用、不动、尾部与孔洞。失败/重试不能读到旧 attempt 的残留。禁止每拍先全量复制 Current 到 Next。

存储块与调度票据分开：一次领取最多 16 个互不重叠的块切片，锁外逐块求值并完整
join。领取粒度按本拍 Active 数和参与线程数选择，避免稀疏物理跨度吞并仅有的并行
任务；128 行存储、块内数值公式和复杂路径判定保持独立。暂存票据使用固定数组，
不在求值阶段增加可失败的堆分配。

计算失败附规范逻辑位置和车内检查位置；完整 join 后，协调器在既有真实 reserve、资源检查与运动检查的位置消费。不能按完成顺序、错误枚举或 `min(rank)` 直接返回；较早真实停车预留失败仍可优先于较晚预计算错误。发现更早未完成义务时须补齐或报执行不变量违例，禁止跳过成功值前缀。

P6 完成验证、生命周期及全部必要容量准备后，CommitPlan 接收完整 Next 运动列与稀疏副作用。P7 发布已验证列所有权并更新目录、资源、日志、事件、时钟和游标，无新增可恢复错误/必要增长。稳定上下文不随运动列交换；P7 资源义务仍有成本，不声明整个提交 O(1)。

目录和活动列在安装/恢复/切换边界预留。非活动记录与控制记录按实际使用量增长；
P6 最后准备新冷记录所需容量，失败返回 `StepError::VehicleStorageAllocFailed`，
不能提前覆盖既有逻辑错误或修改已提交权威。生成/替换使用同名错误，停车和切换
分别沿用 `AllocationFailed` / `StagingAllocFailed`。P7 只处理已冻结的规范控制变化。

## 5. 后续阶段

P1 occupancy 保留精确区间、循环 occurrence、跨边车身、零进度点、自排除及 suffix 的两个不同 owner；改变布局不降级为简单下一辆车。P3 候选使用资格掩码、紧凑工作集与有序两个不同 owner 的分段摘要；删除 owner 必须保留贡献或重建。P4 先在同一规范候选内部批量检查资源，owner/generation/配对/容量/序号仍为完整权威。独立组件裁决不是本轮前置。

## 6. 交付与验收

先交付 A（唯一列式权威和目录）+B（共享基础输入）+C（完整纵向内核）的可运行组合；不要求 A 单独胜过旧布局。D 为 P1/P3 批量化，E 为 P4 资源检查及 P6/P7 扩展；所有阶段逐项记录实际完成边界，不把原型或单个内核记为整体完成。

必须覆盖相同输入下同布局三后端、旧基线、worker 1/2/4/8/16 的状态/决策/事件/首错；真实 reserve、无半提交、重试、generation、恢复、切换、尾块/孔洞、溢出/nonfinite、亚毫米和多 hop 均有反例。生产回归、Clippy、格式、依赖及 CI/CodeQL 门禁保持。

成本账报告实际基础查询、提案/投影调用、有效通道/密度/回退、列读写/中间物化/栈帧、规范消费/资源/归并/发布；普通 release 以完整矩阵报告 mean/p95/p99、稳定 Active 与资源账本。诊断和普通计时分开；所有争用、波动和失败保留。完整源码、构建身份、原始结果与验封在可下载冻结归档中，Git 只留源码/方案/结论/小型索引。

十万总车辆的短窗不等于持续十万 Active 或 #707 达标。16 ms 是原目标；70% 覆盖/2.5 倍的成本模型是假设，不能写成收益。
