# 零非入口 Gate 快路径的有限生产对照

#622 仅在 `non_entry_count != 0` 时执行第二遍 Gate 决策生成。首遍计数、溢出与
容量处理不变；已有 Waiting 决策排序和 `stage_transition_events` 仍执行。
不修改零事件快路径、Waiting/Conflict 规则、公开 API、格式、依赖或状态布局。

## 结论

**建议采纳这个窄快路径供评审：三组输入观察到平均整步耗时下降，10000 / 16
未武装输入尚未证明提速。** 下表为 12 对独立进程的配对变化，负数表示更快。

| 输入（车辆数 / 边数） | 日志   | 时间变化 | 描述性 95% 区间  |
| --------------------- | ------ | -------- | ---------------- |
| 1000 / 256            | 未武装 | -2.43%   | [-4.65%, -0.32%] |
| 10000 / 256           | 未武装 | -4.11%   | [-5.79%, -2.54%] |
| 10000 / 16            | 未武装 | -1.09%   | [-3.55%, +1.71%] |
| 10000 / 256           | 武装   | -2.20%   | [-3.98%, -0.48%] |

第一、四组区间上端接近零；第三组区间跨零，不能证明等效或排除小幅回归。
平均耗时下降不代表尾延迟改善：1000 车辆组的进程 p95 中位数为
408.55 → 413.50 微秒。所有轮次保留，没有追加扩测或挑样本。
本轮只比较 #621 合入后的基线，不与 #620 收益直接相加或比较跨批绝对耗时；旧
画像的 14.45% 不是当前份额或可删除比例。不外推非零 Gate 密集场景、100k 或
其他机器，不完成父任务 #219，也不追加其他热点。

## 验证与方法

- 新回归在旧逻辑下复现零计数仍访问 2 辆车；修复后访问 0 次，非零计数仍按
  live-order 生成。实际覆盖零非入口 Gate 但有 Waiting 决策 / 转移事件、规范顺序、
  有到无空发布、后续事件不变量错误，以及失败零提交和 retry/fresh 三类输出一致。
- Runtime release：455 passed、0 failed、29 ignored；手动 A/B、分配另跑。
  fmt、release check、Runtime 架构检查及生产库严格 Clippy 均通过。
- 两版六组分配输入均暖态零分配 / 重分配；状态摘要和日志字节数一致。
- 原样复用 [#618 的入口及方法](../issue-618-unarmed-journal/README.md)：四组道路
  输入，100 ms 步长；每世界暖机 32 + 8 拍、计时 64 拍，每进程另排除但保留
  两个预定暖机窗口。Individual / Active / Intent 数保持，Presented / Aggregate 为 0。
- 12 对 AB/BA 平衡轮次、输入顺序轮转，共 96 进程、270336 拍正式样本和 12288 拍
  排除暖机样本；全部进程、窗口、摘要和日志预算校验通过。计时不含建世界和输出。
- 主指标为配对进程平均时长比值的几何平均；整对 bootstrap 10000 次、seed 618。
  区间为描述性区间，不把逐拍当独立实验，也不把短窗口 p99 当稳定尾延迟证明。
- 2026-09-09（日本时间），AMD Ryzen 9 9955HX、Windows MSVC、平衡电源计划，
  未锁频 / 绑核、不声称系统空闲；测量期间无本任务 Cargo 构建或 ETW 采样。
  两版显式固定 Rust 1.98.0 默认 release；宿主默认的 1.98.1 未用于生产 A/B。

## 复现

基线 `d4474d2318ecac49cdb3016502148f4d8088a05a`，
候选 `aa13a45d5c98ad082c9bfe30e28f0db65da7954d`；共同测试入口与构建环境一致。
分别构建并将 EXE/PDB 冻结到不同目录，再运行既有脚本：

```powershell
cargo +1.98.0 test --release --locked --offline -p laneflow-runtime --test runtime_profile_evidence --test runtime_profile_allocation --no-run
pwsh -NoLogo -NoProfile -NonInteractive -File research/issue-618-unarmed-journal/run-ab.ps1 -Baseline '<baseline-exe>' -Candidate '<candidate-exe>' -OutputDirectory 'target/issue-622/new-run'
python research/issue-618-unarmed-journal/analyze.py 'target/issue-622/new-run' 'target/issue-622/new-analysis'
```

原始日志、逐拍样本、统计及源码 / 二进制指纹只留本机 `target/issue-622/`，不入库。
其他机器复算本批结果需先取得本地制品；分析器拒绝缺失进程或不一致窗口。
