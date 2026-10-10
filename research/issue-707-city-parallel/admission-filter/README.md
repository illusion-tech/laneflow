# 准入筛选候选的独立诊断

Refs #830。正式候选未采用，结论见[研究结果](../admission-filter-results.md)。

证据包根目录的 `diagnostic.patch` 只适用于候选 commit
`b89f7afa098d6d109229ba2bfc9b5f73573fea23`，不是当前产品补丁。
在该 commit 的独立干净 checkout 中先执行 `git apply --check`，再应用补丁；
使用 Rust 1.98.0、release、`CARGO_INCREMENTAL=0`，构建
`laneflow-urban-harness`，额外启用 `laneflow-runtime/placement-fixtures`。
补丁修改已有 `bounded-profile/timers.rs`；无需新增模块。
保存二进制并记录 SHA256 后反向应用补丁，运行前核对源树干净。
精确命令、源码包、模块、二进制、计划和输入哈希在下载证据包中。

只启用 `FreshAdmission`、`RebuildContenders`、`PublicReplace`、
`CallerCommands` 四个包含式区间，嵌套项不能相加。其他继承的阶段键和 outcomes
没有测量，零值不表示零成本。`PublicReplace` 使用 harness 已有公共调用计时；
`CallerCommands` 还包括命令编排、回收与记账。
每拍另输出一次整数计数与候选缓存容量；没有逐车计时。
计数在每拍命令前清零，排除安装阶段，汇总分开暖机与观察。

本次只运行一次诊断，不纳入普通 ABBA 的性能结论。所有 1536 拍记录与普通版的
状态、命令、事件、展开计划逐字节相同。验证脚本也识别并保留 harness 的原生进度行。
`candidate-proof.md` 保存已撤回实现的 guard/不变量/测试对应，后续方案不能直接继承
未验证的放宽条件。
