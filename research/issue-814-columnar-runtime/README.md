# 列式权威与完整纵向 SIMD 组合验证

对应 #814；设计权威为 ADR 0031 和 `traffic-runtime-columnar-execution.md`。
正式基线为 `cbfbb14a714d819cd5e608a1b759575d378f3d53`。

首个组合包含 Active Block SoA128、完整句柄目录、实际使用的稀疏控制/非活动池、
同拍 P2/P5 基础与未量化提案复用、P5 全纵向数值内核、真实路线的掩码多 hop、
完整 join 后规范首错/真实 reserve，以及 P6 容量准备后的 Next 列交换。
P2 现有实际预览消费者仍以标量调用共享的纯数值定义；P1/P3 和 P4 批量化属于后续。

## 普通整拍方案

- 固定主线 AoS、同布局 Scalar、同布局 AVX2、同布局 AVX-512 四臂。
- Rust 1.98.0，release、locked/offline、默认全局目标特性；workers=4。
- 固定 MIXED-PEAK seed 544，10k/100k 总车辆，初始分别为
  7500/75000 Active、2500/25000 Parked；步长 16/33 ms。
- 每独立进程 256 拍、无预热；1–64、65–256、全窗分别统计。
- 三组平衡序列：`ABCD DCBA / BCDA ADCB / CDAB BADC`，每规模 24 进程。
  全部构建和回归完成后串行采集，记录每个进程 mean、nearest-rank p95/p99 和 Active。
- 先比较各后端数值/逻辑流摘要、snapshot、交通计数和所有输出；保持资源/首错回归。
  不一致时保留证据并停止候选。波动、失败和争用不删除或选择性重跑。
- 对照整拍净收益、各窗变化及同臂波动；内核或布局单项不必独立胜过旧主线。
  不以该短窗声明持续十万 Active、16 ms 达标或 #707 完整认证。

`motion-kernel-evidence` feature 只允许在世界安装边界用 `LANEFLOW_MOTION_BACKEND`
选择 `scalar`、`avx2`、`avx512` 或 `auto`。不支持的 ISA 和非法值拒绝启动；
普通默认构建不读取此变量。计时臂无 Runtime 内部诊断，阶段/工作量诊断独立采集。

诊断版在同一冻结源码上导出协调器时钟与工作量计数；100k 每臂三个进程，256 拍。
它的耗时仅用于定位 P5Dispatch（含完整 join）、规范消费与 WaitingPreview 成本，
不代替普通版分布。实际原始求解/复用、投影、有效向量/物理槽与尾部、降速
occurrence 读取、真实路线步、路线解析入口及成功 profile 查询分别计数；必须与
普通版的状态/决定/事件和交通流摘要完全一致。查询计数覆盖所有阶段，不能直接
当成 P2/P5 单独的节省。共享基础初始 128 B 预算与实际大小单独披露。

Rust 采集器入口：`build-collector <新目录>`，然后用受控原件执行
`prepare <base|candidate> <构建目录>` 与 `run <构建目录> <冻结输入> <新原始目录>`。
`prepare-detail` / `run-detail` 生成独立诊断；`analyze` / `verify` 校验普通结果，
`analyze-detail` / `verify-detail` 额外绑定普通原始目录并检查诊断未改变交通。

原始 JSON、JSONL、日志、输入和构建原件全部保存在仓库外。交付时仓库只保留本方案、
简短结果及带下载地址/摘要的冻结归档索引；完整复现和验封入口使用现有 Rust 基础设施。
