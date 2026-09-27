# P5 最终运动成本归因

Refs #772。基线固定为 `05c505dde3b76fde0a5302a567bf46fdb236146f`，
交付研究工具、可复算证据和一个后续候选决策。正式 Runtime 未接入诊断或候选。

Rust 工具为 `laneflow-p5-cost-research`，复用 #768 的采集、源身份与完整文件
校验，保留 `laneflow-p2-cost-research` 的历史协议。冻结输入为 #707 `4de40e04`
外部包，MIXED-PEAK、seed 544、workers 4、warm-up 0、observe 256。
每规模依次 plain/detail/detail/plain/plain/detail，共 12 个串行进程。

## 测量边界

P5Discover、Slots、Dispatch（含 join）、Consume、Fused 是协调器墙钟。
工作线程仅在 `active_index % 64 == committed.tick_index % 64` 时计时，
每 64 拍轮转覆盖全部位置；全量路径计数另外记录。SampleTotal 包含输入、
停止约束、复用判定、重算、到达处理和空计时器，重算内另分运动原语。
子段嵌套，不能重复相加。抽样累计是各工作线程经过时间之和，既不是协调器
墙钟，也不是精确 CPU 时间；不能乘 64 宣称整拍成本或净节省。

ClockFloor 记录空计时器经过时间，提示小段分辨率限制，不机械扣除。
未抽样车辆只更新线程本地计数，块结束后汇总。全量计数与诊断均会扰动运行，
仅 plain 用于短窗性能描述。城市 Active 投影稠密且分发规模至少 1024；
校验器明确拒绝不符合该固定研究协议的输入，不能视作通用 Runtime 验证器。

校验包括路径分区、horizon 复用/计算、抽样数与轮转、时钟嵌套、完整文件摘要、
进程 UUID、HEAD/clean 与交通日志/原生结果一致性。零移动、硬停止、同边与跨边
分别计数；信号与拒绝门计数指通过 reach 跳过后的实际访问，未宣称遍历总次数。

## 复现

在干净且包含工具提交的 checkout 执行，输出使用新路径，保留全部失败与轮次。

```text
cargo build -p laneflow-post-p3-research --bins
laneflow-p5-cost-research prepare plain <new-build-root>
laneflow-p5-cost-research prepare detail <new-build-root>
```

用 Rust 1.98.0、release/locked/offline、`CARGO_INCREMENTAL=0`，分别在导出
树构建 `laneflow-urban-harness`，每树指定独立 `CARGO_TARGET_DIR`，复制 exe
至 `<new-build-root>/plain.exe` 和 `detail.exe`。全部构建结束后再采集：

```text
laneflow-p5-cost-research run <new-build-root> <frozen-input-root> <new-raw>
laneflow-p5-cost-research analyze <new-raw> <new-results.json>
laneflow-p5-cost-research verify <new-raw> <new-results.json>
```

原始包与隔离导出树保留在本地，提交的 JSON 绑定完整摘要。
#707 最终性能和长期交通质量认证另行验收，不作为本研究或后续优化的默认门禁。
