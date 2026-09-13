# #285 复杂路口规模取证

该程序在一个正式 LFCA 路网、一个 `TrafficWorld` 中运行参考路口网格，复用同一
policy、车型、几何和 catalog 绑定入口。32/320 个独立路口分别容纳 10,000/100,000
辆标准车；每路口 312/313 辆，按已有 portal lane 槽位轮流生成，初速为零。
64 段有限路线接续保留真实机动门和 Waiting/Conflict 资源。该负载不命名为
`LF-SYNTH-v1` 或 `LF-CN-URBAN-v1`。

计时之前提交来源，再运行 `freeze.ps1`，输出必须使用工作树之外的新目录。
它对该干净提交执行受控 release 构建，
从 Cargo 的编译制品记录取得二进制并复制到结果包，再用所复制的生成器从同一份
检入配置生成两档输入；不接收外部网格或预先构建的测试程序。
冻结前核对实际 cells/车辆数/固定步长、两档配置摘要以及构建前后的来源提交。
生成入口先解析并校验源配置的 16 ms 步长和 72,048 ms 信号周期；改变时制须先更新
取证协议，不能用程序内部常量代替对源配置的核对。
结果包保留构建日志、编制配置、车辆/路线命令摘要、完整初态快照、seed、二进制
摘要及环境；`prepared.json` 与 `prepared.initial.lfrs` 都进入冻结文件清单，并交叉
核对快照摘要。runner 与分析器均拒绝快照缺失、损坏或未纳入冻结的结果包。
每个进程启动前重新核对二进制摘要。
runner 开始时复核硬件身份、OS、BIOS、CPU/内存与 GPU 驱动的稳定摘要；每轮前后
记录电源计划和供电状态，与冻结时不一致则拒绝该轮。充电状态原样记录，不与 AC
供电状态混淆；厂商性能模式未测量仍明确标注，不由摘要推断。
正式运行期间不得重建二进制、修改输入或并行运行其他重负载测量。

```powershell
./research/issue-285-junction-scale/freeze.ps1 -OutputDirectory <new-evidence-dir>
./research/issue-285-junction-scale/run.ps1 -EvidenceDirectory <evidence-dir>
<evidence-dir>/bin/junction_scale_analyze.exe <evidence-dir>
```

最长信号周期为 4,503 个 16 ms tick。每进程预热 18,012 tick，观察 36,024 tick，
保留 H/2H/4H 摘要。三个独立非插桩进程使用 `[0,1,2,4,0]` 外层输入量子序列：
采用性能基线的 canonical 2 步上限，4 量子的输入留下 2 量子 backlog，由下一帧补完。按实际步数分类
统计，逐帧核对 backlog，报告恢复帧数；边界处截短帧保留在原始数据中。
十万全量提取后，按稳定调用方车辆身份
选取前 10,000 辆应用 Transform；一万全部应用。offscreen renderer 用 1600×1000
目标和无光照车辆方块，检查所有应用对象均进入视图，报告 GPU/驱动以及渲染提交到
同步 GPU 完成的耗时。这不是道路美术或游戏渲染预算，也不是 GPU timestamp。

各组件 percentile 来自同一 integrated run。`spatial_adapter` 由同一帧的 pose 与
Transform 样本相加后统计；领域观测另列。完整帧明确含证据收集开销，不把不同运行
的 percentile 相加。tick 计时包围正式 LaneFlow Step，包含其驱动开销。
`laneflow_frame_without_evidence` 从同一帧原始墙钟扣除该帧实测 renderer 和取证子
区间，保留领域观测、ECS 调度及帧驱动；全部原始项均保留，可重新计算。
另跑完整观察窗口的 allocation 程序，只校验 allocation/reallocation，不引用其延迟。
持续 Waiting membership/Conflict reservation 的车辆 tick、持有时长，以及每车反复
申请次数用于限定实际负载。申请计数仅纳入 `Granted`/`NoGrant`，多次取得资源另列；
不能由每次观测、决策总行数或总车数推断高并发仲裁。
持续持有要求 Waiting 与 Conflict 的最长连续持有均至少为 2 tick；累计车辆 tick
非零或反复单拍取得资源不能替代该条件。
Waiting 队列中的多次申请可以来自不同车辆，因此分别列出每等待区请求数和同车
重复请求数；不把不同车依次进入写成同车反复进入。

预热和观察窗口的每一拍都由示例私有检查器核对已提交整数几何、完整车身跨边重叠、
follower 最小净距（对合法但不足配置净距的旧状态保持其原有净距，不继续缩小）、
固定身份/路线/生命周期、无停车命令条件下的 binding、tick/time、冻结相位程序与
snapshot(T) 下的实际过门指示、事件顺序/过门数量/资源因果及 Conflict 排他性。
首次等待入口硬停接触与同车的准入决定共同推导应有的 Projection 集合，须与实际事件
精确相等，原因一并核对；漏报也使运行失败。本拍开始时持有或本拍新取
得的 Conflict zone 都保留其 owner 到整拍检查结束，释放事件不能让另一 owner 当拍复用。
资源门 crossing 必须有其精确 occurrence 的 reservation 或 Waiting entry/member
authority；Waiting 事件还核对静态 entry/release、物理顺序、连续准入 counter、
逐区 occupancy/capacity。reservation 的 `acquired_tick` 与实际取得事件一致且不漂移，
完整静态路径（含无门路径）推导每拍应有的 completion 集合，并与事件批次精确比较，
不能漏报。全部事件核对实际路线中的机动出现项、语义 hop 和规范触发位置；车尾事件
使用静态 passage 出口加实际车长推导的位置。运动同时检查固定步长的位移与正速度
增量上界；速度允许一个 mm/s 的转换舍入差，硬停减速仍合法。
Conflict entry/clear 比较完整 passage locator，且必须是实际车头/车尾的首次跨越；
拍末 claim 的 entered 标志和保留状态也须匹配位置，漏报 entry/clear 同样失败。
每份拍末 reservation 必须仍有未清空的 claim。Waiting membership 从入口保存精确的
`release_hop`，离开事件、公开状态和拍末位置均须匹配，不能拖到更晚的重复出现项。
traversal 的路线、出现项和阶段由静态路径、过门位置、当前资源及该拍信号归因推导，
逐车比较；已完成出现项不能继续保留，Waiting 与 Clearing 的资源/静止条件也须成立。
当前状态的内嵌句柄与 live 身份、上一拍身份同时核对。
`carry_um` 是待落地余数，硬停清零不视为已提交位置倒退。检查器只读取公开状态，
不生成通行决定；失败立即使候选进程失败，保留 `.validation-failure.json` 和可捕获的
失败快照。成功 JSON 记录完整检查计数和各类零违规值，分析器拒绝缺失、少查或有
违规的运行。该冻结计划不执行停车/替换/销毁/失败重试命令；这些适用条件由独立
有限验收矩阵覆盖，不声称本规模窗口再次测试了它们。
每拍每辆车的位置、速度、traversal 和 membership 进入完整轨迹摘要，三轮和分配运行
必须逐拍轨迹一致；领域决定与事件继续按每拍完整批次散列。该编码仅属于取证程序。
准入决定在规模进程中核对身份、路线、Gate/zone/passage、顺序、组合拒绝的一致归因
及 grant 与实际资源取得的因果。IIDM、ETA、lead/lag、候选优先顺序与全部 NoGrant
数值理由的独立期望值复用 §5 的有限领域专项和 #645 已登记矩阵；不在此重写第二套
求解器。当前测量对象是唯一 exact 生产实现；后续性能候选须与冻结 exact 来源的逐拍
轨迹和决定对拍，不能把重复性单独当成正确性证明。
`validation.domain_batches_sha256` 覆盖预热与观察的全部决定/事件批次；
`domain_event_digest` 另保留观察区间摘要。分析器要求两者分别一致，不能以预热后收敛
掩盖预热期差异。非空资源门的 Granted passage 必须是该门的规范首项，并与实际取得
范围的首项精确一致；pure Waiting 的无 passage 授予按其静态资源声明核对。
每拍还将 Adapter 领域观测与同一已提交世界逐项对照：完整上下文、共享根、稳定顺序
中的车辆/预约、所有 Waiting zone/member 和完整 decision/event 切片；当前位置、
持有资源和本拍决定/事件使用的 route/hop 定位结果也须一致。记录
`checked_observation_ticks`，分析器要求预热与观察的每一拍均覆盖。
每帧另外由已提交 VehicleState 构造直接 Spatial 输入，对照现有 exact Spatial
基线的完整 pose 批次，再检查冻结的 vehicle/entity 映射与实际 Transform 的位置、
旋转及缩放。全量 pose 和被应用的 Transform 均记录覆盖行数，分析器拒绝缺失或
少查；Spatial 几何公式本身的独立 oracle 仍复用有限矩阵。该核对耗时单列为
`presentation_validation`，从同帧 LaneFlow 成本中扣除，内存保留在进程峰值。

逐拍校验耗时归入取证子区间，从同帧 LaneFlow 成本中扣除；检查器内存计入进程峰值，
不冒充 Runtime 组件账本。结果 PID 必须匹配对应进程记录，记录名称、二进制路径、
输入/输出路径、车辆数、窗口与帧模式必须一致；账本进程的参数及输入/输出环境绑定
也须匹配。不能把两档同一二进制的进程记录互换后继续汇总。

内存和访问账本用独立优化测试进程接续正式运行的 `.warm.lfrs`：设置
`JUNCTION_LEDGER_LFCA`、`JUNCTION_LEDGER_SNAPSHOT`、`JUNCTION_LEDGER_VEHICLES`、
`JUNCTION_LEDGER_CELLS`、`JUNCTION_LEDGER_OUTPUT` 后运行：

```powershell
cargo +1.98.0 test --release --locked -p laneflow-runtime --lib junction_reference_ledger -- --ignored --exact kernel::junction_ledger::junction_reference_ledger --nocapture --test-threads=1
```

该测试保留 H/2H/4H 摘要，须与对应 integrated run 相等；CSV 分列共享根、世界五类
自有存储、scratch、Conflict retained、top-two frontier 和候选/访问/claims/碰撞计数。
子账本有交集，不再与总账相加。它只报告接续 4,096 tick 中的逻辑容量，不是完整窗口
的进程内存或延迟证据。runner 另记工作集峰值、采样 private bytes 峰值和进程 commit
峰值，退出后通过保留句柄再读取 OS 生命周期峰值，覆盖末尾序列化和短进程；
snapshot payload 与 pose 已初始化输出字节另列。Rust 分析程序用显式错误检查校验
全部输入和进程记录，release 构建同样拒绝失败证据，再写入新的 `summary.json`。

按现行设计保留一万 Runtime p95 ≤ 2 ms、十万 ≤ 16 ms，以及 Spatial+Adapter
p95 ≤ 4 ms 的比较，并按性能基线报告 p99 ≤ 1.5×p95、max ≤ 2×p95、Core max
不超过实际 fixed quantum 的尾延迟比较。一万普通单步 LaneFlow 帧比较 6 ms p95；
十万同帧只比较 16.667 ms observation 阈值，无 Product p99/max 门槛。
两档复用参考场景的 16 ms quantum；十万是 stretch observation，不能冒充 33 ms
scale 产品认证。硬件角色、支持的 release OS、产品内存上限或实际预算未满足时
必须逐项标明；不得由这些参考路口研究行宣称产品认证。原始失败记录与未测量项也
必须随结果保留，最终验收取决于独立审阅，不由此程序自行关闭 #285。
