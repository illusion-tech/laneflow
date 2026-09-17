# 已提交位姿缓冲与按需提取

**文档状态**：Accepted（#681；2026-09-17 用户接受，尚未实施）<br>
**最后更新**：2026-09-17<br>
**适用范围**：Traffic Runtime 只读来源、Spatial 批量输出、Adapter 封闭提取与调用方表现选择

相关合同：[Adapter API](adapter-api.md)、[Spatial 几何](spatial-geometry.md)、
[共享静态路网消费](traffic-runtime-shared-consumption.md)、
[修订切换](traffic-runtime-revision-cutover.md)、[城市运行程序](urban-demand-harness.md)、
[ADR 0021](../adr/0021-traffic-infrastructure-and-host-boundary.md)。

## 1. 范围与权威

Runtime 保持全部车辆的交通精度、固定步进、路权、状态与事件权威。是否表现以及表现
哪些车辆由宿主决定；减少采样或 Transform 写入只计作表现收益。Spatial 只绑定完整
不可变根 `Arc<SharedNetworkRevision>`，不依赖 Runtime、车辆句柄或 ECS。

本设计区分三件事：复用来源存储、取消成功批次的再次复制、按显式选择减少采样。
三者可分别取证，不把其中一项的收益当作完整链路收益。停车位缓存不是首批交付要求。
当前的 `committed_pose_sources()` 按值返回和全量封闭提取仍是现行实现；后续
以独立切片替换，不为 1.0 前接口保留别名或兼容双轨。

## 2. Runtime 只读来源

全量读取改成借用世界的惰性迭代器，元素仍为
`(VehicleHandle, PoseSource)`，按 `live_order` 稳定顺序产生。迭代器生命周期绑定
`&TrafficWorld`，持有期间不能步进、提交命令或切换根；它不借用公开内部连续数组。
不承诺 `ExactSizeIterator`，因为 Completed 和 virtual Parked 不产生来源。

增加同一私有判定原语驱动的单句柄只读查询，供 Adapter 选取路径调用：

- 当前有效 Active 返回 Lane；显式 Occupied Parked 返回 Parking。
- 当前有效 Completed 或 virtual Occupied Parked 返回 `None`，不视为移除。
- 不存在或代际失效的句柄返回结构化错误，不静默省略，也不返回复用该槽的新车辆。
- 两个读取入口共用一处生命周期/来源判定；不得让 Adapter 重写停车和道路规则。

Iterator 与单项查询的最终 Rust 类型名由该实现切片冻结。删除当前仅为按值 Vec
封套的 `CommittedPoseSourceBatch`，同时迁移全部调用点、测试和正式文档。Runtime
不存 Adapter 缓冲，不改变快照、交通配置、执行配置或状态摘要。

## 3. 全量封闭提取与缓冲所有权

`LaneFlowSession::extract_committed_pose_batch` 保留全量语义。Session 拥有可复用的
PoseInput 和候选 VehicleHandle 缓冲；调用方拥有可复用的完整输出。一次 `&mut self`
内完成根配对、来源采集、Spatial 采样和输出提交，不向调用方暴露可跨切换重放的
中间输入。绑定完整根的 `Arc::ptr_eq` 检查仍为每批固定 O(1)。

Spatial 在自有 scratch 中完成逐项采样和首错检查；全部成功后交换 scratch 与
`output.records` 的 Vec 所有权，再写入修订、frame 和 placement token。交换不会改变
记录顺序、数值或错误顺序。失败时输出全部字段逐项不变；清空部分 scratch，保留容量。
成功交换后清空接管的旧 output records，保留其 backing 供下次使用。

Adapter 只在 Spatial 成功后提交对应车辆序列和消费上下文；这些最终提交操作必须
不可失败。禁止先更新句柄序列或上下文，再等待可失败的 Spatial 操作。

稳定容量指同一 Session 与 output 反复配对、两侧缓冲均达到所需容量后的零新增
分配。首次调用、首次交换后的另一侧扩容、规模增长、改用新的 output、交替使用多个
小 output 都可能分配。不得声称交换必然降低峰值：双缓冲仍须同时存在；分别记录
len、capacity 和所有者，不把同一 Vec 的转移重复计入 retained bytes。

普通 Rust allocator OOM 沿用现行边界，不新增可恢复分配失败承诺。`Result` 失败
原子性不能冒充进程分配失败恢复；如后续需要资源限额，必须单独冻结预算与诊断顺序。

## 4. 按需封闭提取

增加与全量提取共用内核的选取入口。调用方提供当前消费上下文和一段有序
VehicleHandle 选择列表，列表表达期望顺序；它不是 Runtime 或 Spatial 数组下标。
选择列表不含 PoseSource、几何或进度，来源仍在封闭区间内从当前已提交世界读取。

G1 接受的接口边界如下：

1. 先验证选择上下文与当前世界身份/世代一致，再检查 Spatial 存在及根配对。
2. 重复选择属于输入错误，按选择列表原始顺序报告首个重复；其检查使用可复用的
   受输入规模约束索引，不能每次清零世界容量大小的表。确定完整错误顺序后写测试。
3. 按输入顺序读取当前句柄；stale/unknown 整批拒绝。有效但当前不可表现的句柄
   （Completed、virtual Parked）省略。输出显式列出实际产生来源的句柄。
4. `PoseRecordId` 仍只是本批连续序号，与实际输出句柄一一对应。稳定身份由世界
   上下文与代际句柄共同表达，不能用批内序号、Vec 地址或集合位置跨帧绑定实体。
5. 仅采样实际选出的来源；输出顺序为选择列表省略不可表现项后的顺序。空选择
   合法，成功输出空记录/车辆、当前修订、无 frame 和新 placement token。
6. 任一错误保留整个旧输出，包括车辆、记录、修订、frame、token 与消费上下文。

选择上下文用于防止跨世界/跨切换重放，不保证跨普通 step 或命令仍是旧时刻。每次
调用读取该封闭区间内的最新已提交状态。选择可以在宿主外层帧变化；这不得影响
Runtime fixed tick、交通数量、意图更新数或观察日志。

该模式不执行未选车辆的 Spatial 验证，因此不能替代全量验证路径的首错合同。
例如未选车辆的几何错误由全量审计发现，而选取路径只对本批负责。不得把这种覆盖
收窄藏在现有全量方法里。实现必须显式区分 FullValidation 与 SelectedPresentation
运行模式；它们是不同用途，共用一套生产采样实现，不是新旧兼容分支。

选择收集、重复检查、句柄查询和生命周期处理均计入产品式链路成本。研究中的
“已经拿到 K 个 Active 句柄”探针只说明局部可行性，不能代表最终选取 API 的完整成本。

## 5. 切换、缓存与表现生命周期

- 成功跨修订切换或同修订恢复后，旧选择上下文和旧批次消费资格失效。旧 Arc 的
  存活不授予在新活动世界提交的资格。失败切换继续使用旧根、旧上下文和旧输出。
- Spatial 缓冲仅保存待提交/旧输出记录，不把它当新根的缓存命中；新 Session
  只绑定新根。复用缓冲容量可以作为纯存储优化，但每次新批次必须重新填充。
- placement token 仍由宿主颁发且在应用 Transform 前复核；重放置可以不改根，
  但必须换 token。缓存 canonical pose 也不能绕过这个检查。
- 暂时未选中、Completed 或 virtual Parked 不等于 despawn。宿主保留个体身份与
  代际绑定，隐藏/去掉 Transform；真正移除或 replacement 使用既有 typed 事务。
  不可把“本批缺席”解释成删除，也不可只更新本批就遗漏先前可见实体的隐藏。
- 绑定维护分别测量首次创建、稳定复用、选择进出、parking 隐藏/恢复和真正移除。
  全量 live HashMap 的重建可以研究替换，但持续映射仍须核对生命周期与身份；
  单纯查询 K 个已绑定句柄不能证明完整维护等价。

显式泊位的 canonical pose 只由根内不可变几何与 ParkingSpaceOrdinal 决定，理论上
可缓存；缓存值不含 PoseRecordId、车辆、frame placement 或 world generation。
若后续采用缓存，必须由绑定根的 Spatial owner 持有，根替换时清空/换 owner；另定
有限容量、冷启动成本、稀疏访问和多 Session retained bytes，避免按全部静态泊位
无条件分配。本次只测热重复采样与缓存复制下界，不据此交付缓存。

## 6. 验证与计量

保留城市 harness 当前全量路径、完整 oracle、全部交通校验与逐 tick 摘要。产品式
选择作为独立运行模式，明确 N_individual、N_active、presentable、requested、extracted、
applied；`N_presented` 按 glossary 现行定义，不把 applied 与 extracted 混写。
全量模式在十万档可以 applied=extracted/10，但 N_presented 仍包括已提取个体。

测量必须分开来源查询/分配、选择、采样、成功复制、Transform 转换、绑定维护、应用
与验证。记录真实大小、容量、分配/重分配次数、逻辑复制字节和计时边界；逻辑复制
字节不等于实测内存总线流量。分配计数构建与正常墙钟构建分离，至少三个独立进程。
初始化、编译 LFCA 和正确性断言不混入热路径时间；冷成本另列。

最低正确性矩阵包括：

| 边界                                                | 必须验证的结果                                       |
| --------------------------------------------------- | ---------------------------------------------------- |
| 全量/选择 100%、10%、1%、0%                         | 所选句柄及规范 pose 与全量 oracle 对应；交通快照不变 |
| 输入重排/重复/stale/槽位复用                        | 约定顺序和首错；新 generation 不接管旧绑定           |
| Active、显式 Parked、virtual Parked、Completed      | 来源成员资格与输出句柄对齐；缺席不等于移除           |
| 首/中/末条失败、混 frame、空批、失败后重试          | 完整旧输出不变，恢复后的结果正确                     |
| 不同世界、成功/失败跨修订、同修订换根               | 配对与上下文失效正确，旧输出不能应用新世界           |
| placement 变化、可见集合进出、typed replace/despawn | stale token 拒绝；持久身份和隐藏/解绑正确            |
| 两侧暖机、规模涨落、交替 output                     | 分配/容量口径准确，无丢失或重复 retained 计数        |

## 7. 独立交付切片

1. Spatial 成功提交改为 Vec 交换：内部局部优化，现有 API 和错误语义保持；独立
   验证全部输出原子性、交替输出和容量增长。可不依赖 Runtime API 迁移。
2. Runtime 借用来源与 Adapter 全量缓冲迁移：依赖本设计 G1，使用同一来源原语，
   一次迁移所有调用方；验收稳态 source 分配消失、原有首错和全量输出等价。
3. Adapter 选取入口与产品式 harness：依赖第 2 项及本设计 G1；引入完整选择验证、
   失效语义与真实生命周期矩阵，保持全量验证模式。重新测量完整产品式链路。
4. Parking pose 缓存：暂缓实施。须先在真实显式停车比例与多 Session 工作负载上
   比较冷/热收益、缓存内存和失效成本，再决定是否立项。

第 1、2 项没有实现依赖；第 3 项消费第 2 项。性能改进的采纳结论与实际完成状态
记录在 GitHub；具体测量样例和局限见 research 报告，不将微测量写成产品达标事实。
