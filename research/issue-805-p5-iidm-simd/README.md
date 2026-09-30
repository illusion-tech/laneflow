# 四车批量布局与 IIDM SIMD 短窗研究

对应 #805，冻结正式基线为 `fcd803f2cb28ba7a94f3f0ff589e7e9b7bdbd7d3`。
比较当前 P5（base）、四车批量标量（layout）与相同布局 SIMD（candidate）。
候选仅写入独立 Git 导出树，正式 Runtime 不改动。批量暂存使用工作线程栈；
保持 Active 完整句柄、预览复用、成功值前缀、规范首错、完整 join、
停车真实预留与一次原子提交。融合与预览继续使用标量原语。

执行逻辑位于 `research/issue-762-post-p3-hotspots`，新增入口为
`laneflow-p5-simd-research`。安全 SIMD 使用固定 `wide = 1.5.0`；
Rust 1.98.0 稳定工具链，默认目标特性，不启用全局 AVX2、fast-math 或 FMA。
仓库保持 `unsafe_code = forbid`。四车批次通过 `f32x4` 运算；需要检查生成代码
实际含 packed 指令。无效、非有限、零目标速度和非正车距沿用逐车原语。

## 采集前冻结方案

- 普通 release，locked/offline，workers=4；MIXED-PEAK，seed 544。
- 一万总车辆初始 7500 Active，16 ms；十万总车辆初始 75000 Active，33 ms。
- 每进程 256 拍；全窗 1–256、进入窗 1–64、筛选窗 65–256 分列。
- 每规模三组六进程：ABC CBA、BCA ACB、CAB BAC。A=base，B=layout，C=candidate。
  每臂每规模六进程，共 36。三个构建及回归检查全部结束后串行采集。
- 每进程先计算均值及 nearest-rank p95，之后才平均进程统计。
  报告全部轮次及 Active 范围，不以 pooled tick p95 或局部算术时间替代整步。
- 十万筛选窗 candidate 相对 base 的三组均值均改善、平均改善超过所有臂最大
  组内同臂均值相对跨度，且一万平均进程 p95 回退不超过 2%，才列为后续接入候选。
  SIMD 相对 layout 的方向单列；不要求每项优化独立改善 2%。
- 若数值、交通结果或失败原子性不一致，保留证据并停止对应候选。
  未满足性能条件则停止本轮接入，不追加正式长窗。

## 来源与复现

复用 #801 受控采集器/Runtime 构建：从干净提交的 Git 归档构建实际采集器，
绑定源码索引、命令、清空后环境、Rust/MSVC 工具链、日志和 EXE；拒绝所有
Cargo 配置搜索位置中的环境配置及旧产物。三个 Runtime 目标目录各自全新且
位于源码树外。构建前后重新检查源码与原生工具链。

```text
laneflow-p5-simd-research build-collector <new-tool-root>
<tool-root>/laneflow-p5-simd-research prepare base <new-build-root>
<tool-root>/laneflow-p5-simd-research prepare layout <build-root>
<tool-root>/laneflow-p5-simd-research prepare candidate <build-root>
<tool-root>/laneflow-p5-simd-research run <build-root> <frozen-input-root> <new-raw>
<tool-root>/laneflow-p5-simd-research analyze <raw> <new-results>
<tool-root>/laneflow-p5-simd-research verify <raw> <results>
```

分析重派生全部三臂交通结果、计时与来源证据；拒绝缺失矩阵、混合构建设置、
进程争用记录、错误源码/EXE/日志和不一致结果。原始证据封存为 zstd，
正式性能认证、长期交通质量和最终资源基线仍由 #707 负责。
