# 已提交位姿提取研究

关联 [#681](https://github.com/illusion-tech/laneflow/issues/681)。生产基线固定为
`0fb7024c76a99aea987afa956aadb98001199659`。本目录是独立 workspace 的研究程序，
生产 crate、现有全量城市 harness 和根 workspace 成员均未修改。

## 结论

采纳三个独立实施切片：Spatial 成功批次交换（[#711](https://github.com/illusion-tech/laneflow/issues/711)）、
Runtime 借用来源与全量缓冲复用（[#712](https://github.com/illusion-tech/laneflow/issues/712)）、
按需提取和产品式 harness（[#713](https://github.com/illusion-tech/laneflow/issues/713)）。
前两项互不依赖，选取入口消费借用来源/单项查询。正式合同见
[已接受设计](../../docs/design/committed-pose-extraction.md)。设计接受不代表生产优化已交付。

停车位缓存暂缓：128 个固定显式泊位的重复采样可省计算，但这一热微测量没有覆盖真实
显式停车占比、稀疏访问、多 Session 内存与冷构建，不能据此承诺值得增加缓存。

## 输入与方法

- 真实 compiler → LFCA → checked shared revision → Runtime install；256 条 8000 m
  平行车道、同一 canonical frame、128 个静态显式泊位，无信号/路权资源。
- 10000 / 100000 辆实际 Active 车辆，经生产 spawn 在各边按 10000 mm 间距布置，
  world 全量来源与 live 顺序一致。输入是固定已提交状态，计时窗口不 step。
- 两档均使用同一 LFCA，字节长 228340，SHA-256：
  `c19994c4cb7bb3e4ec182db5fb26300689efef2b0d64922a83822a692f6dbf83`。
- 每档三个独立正常进程，每 case 暖机 4 次，7 个样本，每样本 32 次调用；parking
  每样本 512 次。每进程每档 175 行，共 1050 行正常墙钟；分配构建每档另跑一次，
  共 350 行。所有原始 CSV/log 与环境记录保存在 [evidence](evidence/)。
- 先对每个样本除以调用次数，再取每进程 7 个样本的中位数，最后取三进程中位数。
  [results.csv](results.csv) 保留三轮值；不池化样本，不把分配构建耗时用于结论。
- 正常 release opt-level 3、debug 1；Rust 1.98.1 / LLVM 22.1.8、MSVC target，
  Windows 29661、AMD Ryzen 9 9955HX、平衡电源方案；进程固定 logical processor 16。
  未锁频、未测温度；case 固定顺序而非 A/B 交错，因此局部差值不等于完整产品加速比。
- 编译、spawn、初始快照、oracle、断言、输出与初始化不进入阶段计时。宿主选择列表
  在计时外预置为前 K 个有效 Active 句柄；选取探针计时含逐句柄查询、输入构建及
  完整生产 Spatial 提取，不含未来选取入口的上下文/重复验证、可见性判定和隐藏维护。

## 逐阶段结果

下表单位 ms/调用；只有对应阶段可比较，不得直接相加冒充同一次全链路。固定顺序、缓存和频率差异也会让单独 Spatial 计时高于包含它的 Adapter 计时；本表不用于阶段占比相加或严密收益归因。

| 阶段                | 10000 辆 | 100000 辆 | 计时范围                       |
| ------------------- | -------: | --------: | ------------------------------ |
| Runtime 按值 source | 0.162291 |  3.000678 | 生产查询、收集及释放 Vec       |
| Spatial 全量        | 2.910956 | 12.852588 | 全部采样和成功复制             |
| Adapter 全量        | 2.477519 | 14.849241 | 生产封闭提取（包含前两类工作） |
| 仅 records 复制内核 | 0.009362 |  0.156831 | 已有记录到复用 Vec             |
| 选取探针：100%      | 1.593769 | 14.387428 | 已知 Active 句柄查询与采样     |
| 选取探针：10%       | 0.152888 |  1.564191 | 同上，1000 / 10000 条          |
| 选取探针：1%        | 0.014175 |  0.143622 | 同上，100 / 1000 条            |

十万档 Transform 转换在全量后选择 10% 时仍为 10.583503 ms；先选再转换的一万辆
为 1.080509 ms。这里是实际 Bevy `Transform::looking_to`，没有渲染器、GPU 或
窗口；不是 Traffic Runtime 加速或城市帧率认证。

Vec 交换内核只交换所有权，在十万档测得约 6 ns/调用，接近该微内核的测量下限。
它不包含采样、header 提交、失败检查及整体适配器工作，不能用复制/交换之比宣称
Spatial 加速倍数。生产交换实现尚未存在，收益以 #711 的完整路径 A/B 为准。

## 分配、复制与存储

本构建 `size_of`：来源 tuple 20 B、PoseInput 16 B、CanonicalPoseRecord 40 B、
VehicleHandle 8 B、Transform 48 B。它们是当前目标布局，不是公开 ABI 承诺。

| 阶段/规模             | alloc/调用 | realloc/调用 | bytes_allocated/调用 |       逻辑 payload |
| --------------------- | ---------: | -----------: | -------------------: | -----------------: |
| source / 10000        |          1 |           12 |             327680 B |           200000 B |
| source / 100000       |          1 |           15 |            2621440 B |          2000000 B |
| 暖机 Spatial / 100000 |          0 |            0 |                  0 B | 成功复制 4000000 B |
| 暖机 Adapter / 100000 |          1 |           15 |            2621440 B |   包含 source 分配 |

`stats_alloc 0.1.10` 的 bytes_allocated 包括初始分配加正向 realloc **增量**，不是
每次 realloc 新容量的总和，也不是峰值 RSS；bytes_reallocated 另列净增长。上述
source Vec 单调增长，因此可推得最终 backing 分别为 16384 / 131072 个 tuple。
这修正了把 source 成本简写成“每批一条 N 尺寸分配”的估计：确实只有一个 Vec，
但过滤迭代器的 collect 还发生多次容量增长。未测 allocator 移动次数或物理复制字节。

空间 scratch 与 output 仍各自持有记录缓冲；交换消除的是成功复制，不消除双缓冲。
record 的逻辑复制量为 N × 40 B，不是硬件内存带宽计数。完整进程/世界 retained
内存及冷启动分配不在本研究证据范围，不填零或用逻辑 payload 代替。

## 绑定与停车

十万 live、一万绑定的分解测量：

| 探针                                      |  ms/调用 | 稳态申请                    |
| ----------------------------------------- | -------: | --------------------------- |
| 重建全量 live HashMap + 核对绑定          | 5.919378 | 1 次分配，2228240 B         |
| 仅核对保留绑定与有效 handle               | 0.658222 | 0 次                        |
| 真实 ECS 移除/插入 Transform 的可见性切换 | 1.442956 | 每 32 调用仍约 1 次 realloc |

第一项复现城市 harness 的全量 live 集合构造和映射核对工作形状，使用真实生产
Session 绑定；并非完整 `Presentation::sample` 计时。第二项省略全量集合和清理，
只能定位局部成本，不能当等价替换。第三项交替隐藏/显示同一批实体、不销毁身份，
ECS 的内部工作仍可能增长；本报告不宣称它稳定零分配。真实移除、replacement、
virtual parking 和选择进出组合留在 #713 验收。

128 个互异静态泊位在同根上重复采样约 0.049206 ms/批；预计算记录复制内核约
0.000045 ms/批。缓存 payload 是 5120 B 的 record（含研究用固定 record id）；
未来生产缓存必须只存 canonical pose/frame，不能缓存车辆或批内 id。本探针不表示
100000 辆停在 128 个泊位，也不计作 Runtime 已占用泊位。未对实际 parked population
做城市测量，故缓存实施暂缓。

## 正确性与局限

- 每个独立进程逐条比较完整 Adapter 和单独 Spatial 输出，包括 batch header、token、
  record id、车辆与 pose；1%、10%、100% 选择均与完整 oracle 的有序子集相同。
- 采集前后完整 Runtime snapshot 相等；末条非法进度使旧 Spatial 输出所有字段
  保持，去掉坏输入后重试与原批次相同；parking 冷输出与重复采样输出相同。
- 41 个既有 Spatial/Bevy 测试通过，包含修订切换、同修订恢复、旧消费上下文、
  typed parking/replace 与生产封闭提取。这证明现行合同仍成立，不代替未来新 API
  的 stale/重复选择、空选择、混 frame、多 output 等完整验收矩阵。
- Runtime lifecycle 另外 14 个测试通过。汇总脚本验证完整输入，并拒绝缺失样本、
  重复样本和缺失 oracle 成功标志；四项脚本检查通过。
- fixture 只测 Active 的大规模完整路径和独立静态泊位采样；不是正式城市工作负载，
  不覆盖不同活动率、停车比例、真实可见性算法或多 Session 缓存压力。
- 所有探针只留在研究程序。没有实现新的 Runtime Iterator、Spatial swap 或选取
  API；不存在已交付生产收益或 #537/#539 达标声明。

首次取证因沙箱 CIM 权限在测量前停止；随后绑定探针错误地对未绑定对象调用 unbind
而中止。修正探针并重新完整构建/跑三轮后才生成本目录 evidence，不混入中止的样本。

## 复现

在仓库根使用 PowerShell 7、仓库工具链和已缓存依赖：

```powershell
cargo clippy --locked --offline --manifest-path research/issue-681-pose-extraction/Cargo.toml --all-targets --all-features -- -D warnings
pwsh -NoProfile -File research/issue-681-pose-extraction/run.ps1 -Output target/pose-681/new-run
pwsh -NoProfile -File research/issue-681-pose-extraction/analyze.ps1 -Evidence target/pose-681/new-run -Output target/pose-681/new-results.csv
cargo test --locked -p laneflow-spatial -p laneflow-bevy --tests
```

run 脚本使用新目录、检查每个进程退出码，保留两种 binary 与 SHA-256；读取 CPU 和
电源信息需要宿主权限。硬编码 logical processor 16 是本次机器条件，换机器须显式
调整并保存环境，不悄悄复用本报告硬件描述。Git 基线与新增研究源/lock 摘要共同
标识本次取证输入；不是声称研究代码已存在于该基线提交。

analyze 要求三个完整进程、每 case 七个不同样本和每进程 oracle 成功标志；任何
缺失都拒绝汇总。原始 CSV 中正常构建的 allocation 四列为占位零，表示未启用计数，
仅 allocation 构建的对应列是测量值。
