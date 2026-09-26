# 同拍 P3 可达性缓存验证

Refs #763。固定基线 `d6f5eee9e53f4a110dd6dcab64a4ab4458e5e426`；候选使用可达、
干净的源码提交导出。目标是验证把保守可达性查询移到 P2 后的整拍净收益。
它不改变算法、候选集合、资源生命周期或 1_024 分发阈值。

本批 24 次无插桩与 12 次诊断已完成，采用结论和不确定性见 [结果](results.md)。

## 本批测量合同

- 复用 #707 `4de40e04` 冻结输入，10k/100k，workers=4，各运行 256 拍。
- 无插桩两臂各尺度三组 ABBA（base/candidate/candidate/base），24 个独立进程。
  Runtime 保持源码原样，只在 Harness 的公共 step 计时结束后打印逐拍时间。
- 单独诊断两臂各尺度三组 A/B，共 12 个独立进程。协调器批次墙钟包含完整 join，
  不累加 worker CPU；P2Independent 是独立预览的发现/分发/消费整体，嵌套于
  WaitingPrepare。P3 子段嵌套于 ConflictPrepare，不能再次累加到整拍。
- 诊断的运动调用、保守查询及缓存命中先在线程本地计数，每块结束时归约；它仍有
  插桩开销，只用于归因，不代替无插桩整拍收益。拍末记录缓存、P2/P3 槽位容量和
  四个缓存/分发缓冲的保留字节；不冒充整个工作区、RSS 或分配器峰值。
- 每轮分别给出全窗、1–64 拍、65–256 拍的 mean/p95 和实际 Active。后两者只是
  分窗，不是暖机或稳态声明。每轮尾延迟单列，不把轮间 p95 平均说成 pooled p95。
- 核验源码、manifest/lock、二进制和输入 SHA-256、采集提交/tree、前后 clean/HEAD、
  进程 UUID/退出码、原生完成及逐拍计时。缓存复用不改变算法，因此本项另核验两臂
  交通日志和结果相同；不把这条义务推广到允许行为变化的模型研究。
- 结束条件：完整边界测试、24 次无插桩和 12 次诊断齐全并可复算；三组收益一致且
  能与本批重复波动分离、另一尺度无明确回归才采用。无法分辨或成本转移抵消则
  调整/淘汰，不自动延长成正式认证。本批不执行 #707 44544 拍或 100k 正式长窗。

## Rust 工具

复用已入主干的 `laneflow-post-p3-research` 包，新二进制为
`laneflow-p3-cache-research`。#762 原命令、固定基线和已发布证据保持原义。

`cargo build --locked -p laneflow-post-p3-research --bin laneflow-p3-cache-research`

从仓库根目录执行，以下 `TOOL` 为 `target/debug/laneflow-p3-cache-research`，Windows
加 `.exe`。`ROOT` 使用全新的 `target/` 子目录；`CANDIDATE` 为候选源码提交。

1. 各 arm/mode 导出：`TOOL prepare base plain BASE ROOT`、
   `TOOL prepare candidate plain CANDIDATE ROOT`，detail 同理。
2. 各导出树使用不同 `--target-dir`，设置 `CARGO_INCREMENTAL=0`，以
   `cargo +1.98.0 build --release --locked --offline -p laneflow-urban-harness`
   编译；二进制复制至 `ROOT/base-plain.exe` 等相应路径。
3. 在已提交且干净的工具树运行：`TOOL run plain ROOT INPUTS NEW_RAW`，detail 同理。
4. `TOOL analyze NEW_RAW NEW_RESULTS`；封存后
   `TOOL verify NEW_RAW PUBLISHED_RESULTS` 严格复算，不覆盖原包。

新源码目录、原始包和统计输出必须为新路径。导出索引忽略编译产物；实际导出树
和外部原始包保留在本工作树 `target/`。发布的索引不代表原始包随 Git 分发。

## 正确性覆盖

共享保守原语覆盖近/远门、无门、未知运动范围、非有限步长与跨边零点。新增缓存
测试毒化结果，证明完整句柄/代次、逻辑更新位置及 Active 位缺一不可；跨边零点
仍携带前门回看。空/部分/完整容量在 workers 1/2/4/8/16 上对照完整求值，涵盖
候选位不同于 Active 位、同槽新代次、失败重试。资源用例实际见证取得、跨拍持有
和清空。新路线与新代次从新 C(T) 计算；原有 P2/P3 可选分配失败、首错和失败后
缓存清理用例继续运行。布局及保留容量由诊断和已有资源计账测试共同覆盖。
