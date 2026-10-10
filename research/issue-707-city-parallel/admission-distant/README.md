# 远处限速下降候选的独立诊断

Refs #834。候选未采用，见[结果](../admission-distant-results.md)。
[归档索引](archive-index.md)记录下载包身份与已执行的恢复校验。
证据包 `diagnostic.patch` 只适用于候选 `d7624fb21da11566e8a4a4a624da3e84c45937c9`。
在其独立干净 checkout 中执行 git apply --check，再应用补丁；使用 Rust 1.98.0、
release、CARGO_INCREMENTAL=0 构建 laneflow-urban-harness，额外启用
laneflow-runtime/placement-fixtures。保存二进制与哈希后反向应用补丁，运行前确认源树干净。
完整命令和文件身份见证据包 prepare-diagnostic.py、freeze-diagnostic.ps1 与 measurement-freeze.json。

只计 FreshAdmission、RebuildContenders、PublicReplace、CallerCommands 四个包含式区间；
其余继承的键没有测量。PublicReplace 使用 harness 的公共调用计时，CallerCommands
还含命令编排、回收及记账。嵌套项不能相加，诊断不能混入普通 ABBA。
每拍输出扫描/筛除/预览、空后缀/有限远处命中、回退计数和候选缓存保留容量；
不逐车计时。末两类回退是 motion-proof 的子集，不能重复求和。

唯一诊断与四轮普通版的状态、命令、事件和展开计划逐字节一致。
所有原始行（含 harness 原生进度行）保留；分析脚本仅对该一轮数据工作。
候选的成功性与资源验证对应见 [candidate-proof.md](candidate-proof.md)。
