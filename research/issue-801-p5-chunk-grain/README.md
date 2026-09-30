# P5 分块粒度短窗筛选

本目录对应 #801，父任务 #707。冻结基线为
`cb562bde948b6f96484581b650422f149f6c5787`。候选只在导出研究树中将
P5 `dispatch_threads() × 2` 改成 `× 4`；导出树参与测试相应要求 16 块及 4 步长
起点，真实多线程参与断言保持。当前正式 Runtime 保持原样。
4 个工作线程的活动路径分别生成 8 块和 16 块。未改变领域原语、规范消费顺序、
资源生命周期、Runtime API、数据格式或 Adapter API，无需新增 ADR。

- [结果与停止决定](results.md)
- [24 次普通测试及逐组比较](evidence/plain.json)
- [6 次独立块级诊断](evidence/detail.json)
- [负向校验记录](evidence/negative-checks.json)
- [实际原生编译覆盖隔离验证](evidence/native-env-isolation.json)

## 冻结方案

- 使用 #707 的冻结 MIXED-PEAK 输入，seed 544；10k 为 16 ms，100k 为 33 ms。
- 每个规模的普通测试串行运行三组 `ABBA / BAAB / ABBA`，每组四个独立进程。
  两个规模共 24 个进程；256 拍，workers=4，无预热。
- 100k 单独运行三组 AB 诊断，共 6 个独立进程；不混合普通及诊断耗时。
- 同时报告全窗 1–256、进入窗 1–64、筛选窗 65–256。每个进程先计算拍均值
  和 nearest-rank p95；组内、组间再平均，不以 pooled tick p95 替代。
- **采集前采用条件**：两个规模筛选窗均须三组方向一致、均值改善至少 2%，
  改善超过所有组内同臂两次均值的最大相对跨度，平均进程 p95 回退不超过 2%。
  任一条件不满足则停止此候选，不接入正式 Runtime，也不强行延长 formal 窗口。
- 30 个进程运行前完成全部编译。每个进程前后用 Windows `tasklist` 保存边界
  快照；发现已知 cargo/rustc/测量程序立即停止，并保留已生成的失败目录。
  边界快照不是连续监测，不保证排除短暂或未知系统负载。

## 工具与诊断定义

实现位于 `research/issue-762-post-p3-hotspots`，入口
`laneflow-p5-chunk-research`，协议 `p5-chunk-grain-v3`。执行逻辑全部为 Rust。

使用 Rust/Cargo 1.98.0、release、locked、offline、`CARGO_INCREMENTAL=0`；
`prepare` 从每个 `<arm>-<mode>-source` 的 workspace 受控构建
`laneflow-urban-harness`，使用全新且位于源树外的 target，EXE 保存为
`<root>/<arm>-<mode>.exe`。已有 EXE、source、target 或构建日志均拒绝复用。

- `prepare <base|candidate> <plain|detail> <root>`：从冻结提交导出；任何替换锚点
  不唯一即拒绝；校验构建前后源码索引相同，受控构建并复制实际产物。
  来源记录绑定源码索引摘要、完整构建命令与环境、工具链版本、EXE 摘要及日志。
  本协议的受控构建限定 Windows x64 MSVC。构建子进程先清空环境，再传入记录过的
  系统路径、Rust 编译设置和固定 MSVC 设置；继承的 `CC`、`CFLAGS`、目标/HOST
  变体、`BLAKE3_*`、`CL`/`LINK` 覆盖及 SDK/VC 选择覆盖均不传入。
  通过已锁定的 `cc 1.2.66` 解析 `cl.exe`、`lib.exe`、`ml64.exe`、`link.exe`，
  记录绝对路径、版本信息、文件大小、SHA-256、解析参数及 SDK 路径环境；
  用 `CC=cl.exe` 与首位 PATH 固定 MSVC 分支，并固定归档器及 Rust 链接器。
  构建前后重新解析且全部记录必须相同，任一工具漂移即拒绝产物。
- `run <plain|detail> <root> <frozen-input-root> <new-raw>`：采集器 Git 树须干净；
  保存 UUID、命令、源码/输入/EXE 哈希、退出码、前后 Git 状态和进程快照。
  启动前验证 EXE 与构建记录相符，复制构建日志到原始批次；旧或交换的 EXE 被拒绝。
  两臂的继承编译设置、受控环境及完整 Rust/MSVC 工具链记录必须相同，任一不一致在首个原生进程启动前拒绝。
- `analyze <raw> <new-results> [plain-raw]`：验证完整矩阵、构建来源、身份、原生日志哈希和各臂交通结果；
  重派生普通比较及诊断，结果只能写在原始目录外。
- `verify <raw> <results> [plain-raw]`：重新派生并逐值比较，不覆盖证据。
- 诊断的 `analyze` 和 `verify` 必须传入 plain 原始批次；先完整重派生 plain，
  再比较冻结输入、100k 全部 12 次普通与 6 次诊断的交通文件摘要、初始及末尾计数。
  结果记录引用批次的身份与文件索引摘要；两臂诊断同时偏移也会被拒绝。
  普通与诊断的四个构建设置及工具链也须一致；发布结果记录 `build_settings_equal=true`。

诊断只在每块首尾取时钟并记计算线程序号，完整 join 后读取。workers=4 时
Rayon 辅助线程使用 0–2，参与取块的调用线程使用 3。日志输出在
公共 `step()` 计时外；诊断记录分配与保存仍产生开销，因此只能用于归因。
块级经过时间包括抢占、等待等影响，累计值不是 CPU 时间，也不是整拍墙钟。
`last_two_chunk_end_gap` 是最后两个块的结束间隔，不等同于可移除的等待成本；
`after_last_end` 是最后块结束到分发返回的间隔。区间重叠不证明 CPU 同时执行。

校验器拒绝非整数、错误拍序、缺失/未完成块、分区空洞、越界计时、同一 worker
的区间重叠、阶段时钟嵌套错误、争用快照、构建来源错误、跨臂或跨模式交通结果变化。
正式性能认证、长期交通质量和资源内存基线不在本切片范围内。
