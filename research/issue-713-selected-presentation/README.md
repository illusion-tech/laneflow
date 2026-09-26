# #713 按需表现完整链路取证

生产入口是城市 harness 的 `Presentation` 和 Bevy 封闭提取 API。本目录只编排独立
进程、验证不可变输入与比较证据，不复制交通、选择、采样或实体生命周期实现。

## 复现

先在已推送且干净的源码提交上构建生成器，按 `examples/config/cn-urban.toml` 生成
10k 或 100k LFCA 输入，再用 harness `plan` 冻结 `MIXED-PEAK` 的完整 correctness
或 performance 窗口。不能传入 probe 或缩短前缀；两种窗口分别按原合同完整运行，
分析均排除暖机。完整 correctness 窗口用于本切片的三轮链路对照，不冒充 #544 的
正式 performance 窗口或 #220 产品认证。

```powershell
$env:LANEFLOW_HARDWARE_ROLE = '<实测 CPU、内存及宿主用途>'
$env:LANEFLOW_POWER_ROLE = '<实测电源模式>'
pwsh -NoProfile -File research/issue-713-selected-presentation/run.ps1 -Artifacts <inputs> -Plan <plan.toml> -Output <new-directory> -RemoteBranch codex/713-selected-pose-extraction
node --test research/issue-713-selected-presentation/analyze.test.mjs
node research/issue-713-selected-presentation/analyze.mjs <batch-directory>
```

运行脚本检查远端分支精确指向源码 HEAD，再分别构建并保存正常墙钟、allocator、
内部阶段插桩三份二进制。输出目录应在仓库外或已忽略的 `target/` 下；测量期间
不能编辑源码。原 FullValidation、相同最终集合的全量对照、Selected 三类用途
明确区分。稳定/动态窗口均固定 percent（默认 10）、offset 53、stride 0/137、
reverse true。正常构建各运行三次，诊断构建各运行一次完整窗口；所有进程串行运行，
第二轮反转执行次序。

## 验证口径

- `batch.json` 绑定独立执行 ID、构建命令、二进制与结果摘要；每轮保留源码/tree、
  manifests/lock、LFCA/计划、工具链、硬件、电源、配置和完整逐 tick 记录。
- 分析器拒绝缺轮、重复执行、短窗口、错误构建、失败、来源漂移及制品摘要不符。
  每对逐 tick 比较交通、命令、事件、实际应用数和 Transform 摘要，完整检查点也须一致。
- 选择 API 错误优先级、重排、状态过滤、失效与缓冲原子性由生产合同测试验证。
  正常路径的来源查询、采样和提交细分由独立 profile 构建观察，不与正常墙钟混合。
- `presentation_ns` 包括选择、提取、转换、绑定维护与应用。完整交通 oracle 与表现
  验证保留但单列；不把几个分位数相加。选择仍扫描 live 个体，绑定维护仍扫描历史
  绑定；不声称整条链路 O(K)。
- 分配字段是进程 allocator 在选择至应用区间的 alloc/realloc 与增长字节；不是峰值
  RSS。容量按当前所有者及元素大小记录，HashMap 容量不是底层字节账。
  大→小与交替 output 的容量/零新增分配由 Adapter 定向测试另外取证。

完整关闭还需要结合运行结果报告收益、原全量验证保留情况和实际覆盖的生命周期，
不能仅凭脚本存在或 `analysis.json` 的结构通过宣称 #713 完成。
