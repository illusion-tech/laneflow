# #285 复杂路口规模取证

该程序在一个正式 LFCA 路网、一个 `TrafficWorld` 中运行参考路口网格，复用同一
policy、车型、几何和 catalog 绑定入口。32/320 个独立路口分别容纳 10,000/100,000
辆标准车；每路口 312/313 辆，按已有 portal lane 槽位轮流生成，初速为零。
64 段有限路线接续保留真实机动门和 Waiting/Conflict 资源。该负载不命名为
`LF-SYNTH-v1` 或 `LF-CN-URBAN-v1`。

计时之前提交来源，再运行 `freeze.ps1`。它对该干净提交执行受控 release 构建，
从 Cargo 的编译制品记录取得二进制并复制到结果包，再用所复制的生成器从同一份
检入配置生成两档输入；不接收外部网格或预先构建的测试程序。
冻结前核对实际 cells/车辆数/固定步长、两档配置摘要以及构建前后的来源提交。
结果包保留构建日志、编制配置、车辆/路线命令摘要、完整初态快照、seed、二进制
摘要及环境；每个进程启动前重新核对二进制摘要。
正式运行期间不得重建二进制、修改输入或并行运行其他重负载测量。

```powershell
./research/issue-285-junction-scale/freeze.ps1 -OutputDirectory <new-evidence-dir>
./research/issue-285-junction-scale/run.ps1 -EvidenceDirectory <evidence-dir>
<evidence-dir>/bin/junction_scale_analyze.exe <evidence-dir>
```

最长信号周期为 4,503 个 16 ms tick。每进程预热 18,012 tick，观察 36,024 tick，
保留 H/2H/4H 摘要。三个独立非插桩进程使用 `[0,1,2,10,0]` 外层输入量子序列：
上限为 8 步，10 量子的输入留下 2 量子 backlog，由下一帧补完。按实际步数分类
统计，逐帧核对 backlog，报告恢复帧数；边界处截短帧保留在原始数据中。
十万全量提取后，按稳定调用方车辆身份
选取前 10,000 辆应用 Transform；一万全部应用。offscreen renderer 用 1600×1000
目标和无光照车辆方块，检查所有应用对象均进入视图，报告 GPU/驱动以及渲染提交到
同步 GPU 完成的耗时。这不是道路美术或游戏渲染预算，也不是 GPU timestamp。

各组件 percentile 来自同一 integrated run。`spatial_adapter` 由同一帧的 pose 与
Transform 样本相加后统计；领域观测另列。完整帧明确含证据收集开销，不把不同运行
的 percentile 相加。tick 计时包围正式 LaneFlow Step，包含其驱动开销。
另跑完整观察窗口的 allocation 程序，只校验 allocation/reallocation，不引用其延迟。
持续 Waiting membership/Conflict reservation 的车辆 tick、持有时长，以及每车反复
申请次数用于限定实际负载。申请计数仅纳入 `Granted`/`NoGrant`，多次取得资源另列；
不能由每次观测、决策总行数或总车数推断高并发仲裁。
Waiting 队列中的多次申请可以来自不同车辆，因此分别列出每等待区请求数和同车
重复请求数；不把不同车依次进入写成同车反复进入。

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
p95 ≤ 4 ms 的比较。硬件角色、支持的 release OS、产品内存上限或实际预算未满足时
必须逐项标明；不得由这些参考路口研究行宣称产品认证。原始失败记录与未测量项也
必须随结果保留，最终验收取决于独立审阅，不由此程序自行关闭 #285。
