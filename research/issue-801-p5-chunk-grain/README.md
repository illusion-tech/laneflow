# P5 分块粒度短窗筛选

本目录对应 #801，父任务 #707。冻结基线为
`cb562bde948b6f96484581b650422f149f6c5787`。候选只在导出研究树中将
P5 `dispatch_threads() × 2` 改成 `× 4`；导出树参与测试相应要求 16 块及 4 步长
起点，真实多线程参与断言保持。当前正式 Runtime 保持原样。
4 个工作线程的活动路径分别生成 8 块和 16 块。未改变领域原语、规范消费顺序、
资源生命周期、Runtime API、数据格式或 Adapter API，无需新增 ADR。

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
`laneflow-p5-chunk-research`，协议 `p5-chunk-grain-v1`。执行逻辑全部为 Rust。

使用 Rust/Cargo 1.98.0、release、locked、offline、`CARGO_INCREMENTAL=0`；
每个 `<arm>-<mode>-source` 从其 workspace 构建 `laneflow-urban-harness`，
target 必须放在该源树之外，EXE 保存为 `<root>/<arm>-<mode>.exe`。

- `prepare <base|candidate> <plain|detail> <root>`：从冻结提交导出；任何替换锚点
  不唯一即拒绝；保存修改后源文件索引。
- `run <plain|detail> <root> <frozen-input-root> <new-raw>`：采集器 Git 树须干净；
  保存 UUID、命令、源码/输入/EXE 哈希、退出码、前后 Git 状态和进程快照。
- `analyze <raw> <new-results>`：验证完整矩阵、身份、原生日志哈希和各臂交通结果；
  重派生普通比较及诊断，结果只能写在原始目录外。
- `verify <raw> <results>`：重新派生并逐值比较，不覆盖证据。

诊断只在每块首尾取时钟并记 Rayon worker 序号，完整 join 后读取。日志输出在
公共 `step()` 计时外；诊断记录分配与保存仍产生开销，因此只能用于归因。
块级经过时间包括抢占、等待等影响，累计值不是 CPU 时间，也不是整拍墙钟。
`last_two_chunk_end_gap` 是最后两个块的结束间隔，不等同于可移除的等待成本；
`after_last_end` 是最后块结束到分发返回的间隔。区间重叠不证明 CPU 同时执行。

校验器拒绝非整数、错误拍序、缺失/未完成块、分区空洞、越界计时、同一 worker
的区间重叠、阶段时钟嵌套错误、争用快照和跨臂交通结果变化。
正式性能认证、长期交通质量和资源内存基线不在本切片范围内。
