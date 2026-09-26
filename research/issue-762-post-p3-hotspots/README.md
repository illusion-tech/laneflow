# P3 合入后的剩余整拍热点

Refs #762。固定主干 `37c5e1af3c72b0f60cf2dd889bbf6713b2a77054`，只交付研究工具、
诊断证据及下一优化切片的选择。无正式 Runtime/API/数据格式/Adapter 行为变更。

## 测量合同

- 原 #707 `4de40e04` 冻结包的 10k/100k `*-smoke.toml`，workers=4，每次 256 拍。
- 每规模 plain/stages/stages/plain/plain/stages；各三轮独立进程，测量串行执行。
- plain 保留主干 Runtime 原样；两臂 Harness 均在公共 `step` 计时结束后输出逐拍记录。
- stages 仅激活既有十段协调器批次墙钟，额外区分 ConflictPrepare 内 Frontier 与 P4。
  嵌套两段不得再次加入十段合计。计时涵盖等待 worker 完成；不累加 worker CPU。
- 分别报告全窗、1–64 拍、65–256 拍。这里的前缀分窗不是正式暖机或稳态声明。
- 三轮主要段排名一致且领先超过该段自身运行间均值极差，才据此指定下一原型；
  差异不可分辨就明确保留不确定性，不自动扩展为长测。
- 核验源码/二进制/输入身份、阶段次数、非负残差、两臂交通日志及结果语义一致。
  这是观测补丁的透明性检查，不给后续模型优化附加历史轨迹等价要求。
- Windows 开发机未完全隔离后台活动；正式性能预算与长期质量认证仍由 #707 承担。

## 复现

使用 Python 3.11+、Rust 1.98.0，从已提交干净研究树运行。导出目录必须不存在。
两臂使用**不同 Cargo target-dir**，避免同包同版本导出树的增量指纹误复用。

```powershell
python research/issue-762-post-p3-hotspots/prepare.py target/hotspot-plain plain > target/plain-source.json
python research/issue-762-post-p3-hotspots/prepare.py target/hotspot-stages stages > target/stages-source.json
$env:CARGO_INCREMENTAL='0'
cargo +1.98.0 build --release --locked --offline -p laneflow-urban-harness --manifest-path target/hotspot-plain/Cargo.toml --target-dir target/hotspot-build
cargo +1.98.0 build --release --locked --offline -p laneflow-urban-harness --manifest-path target/hotspot-stages/Cargo.toml --target-dir target/hotspot-stages-build
New-Item -ItemType Directory target/hotspot-binaries
Copy-Item target/hotspot-build/release/laneflow-urban-harness.exe target/hotspot-binaries/plain.exe
Copy-Item target/hotspot-stages-build/release/laneflow-urban-harness.exe target/hotspot-binaries/stages.exe
python research/issue-762-post-p3-hotspots/run.py target/hotspot-runs E:/projects/laneflow-evidence/issue-707/4de40e04
python research/issue-762-post-p3-hotspots/analyze.py target/hotspot-runs target/hotspot-results.json
python -m unittest discover -s research/issue-762-post-p3-hotspots -p 'test_*.py'
```

原始运行包、二进制和导出源码保存在独立工作树的 `target/`，不纳入 Git；远程复核需要
对应外部包及冻结输入。源码索引和原始文件清单用于核对身份，不表示外部证据已随 Git 发布。
