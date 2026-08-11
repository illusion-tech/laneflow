# #296 RoadEditingSource 校准

本包只承载 `LF-ROAD-EDITING-P100-v1` 的 test/research generator、跨语言 fixture 和正式
校准证据，不是 production JSON frontend。production compiler 和 writer 不依赖本包，
也不读取旧 Geometry JSON。

语义种子只有在以下条件全部成立后才可进入生成器：

- `road-editing-source-workload-definition-v1.json` 绑定的 seed 路径与 SHA-256 精确匹配；
- 外层 seed 和每份嵌入 Geometry 文档都通过 `serde_json 1.0.151` 有类型反序列化；
- 所有 DTO 使用 `deny_unknown_fields`，重复逻辑字段由 serde 失败关闭；
- 五份文档独立复算的结构计数与冻结 workload 完全一致。

本阶段不恢复、包装或兼容已经删除的 production JSON parser。

本地入口：

```powershell
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- seed-audit
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-p100 2 2
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-regularity
cargo +1.96.0 run --locked -p issue-296-road-editing-source-calibration --bin calibrate -- road-editing-fixture-identities
```

`road-editing-p100` 通过第一方 typed model 和 writer 生成五个 size-prefixed `LFRE`
buffer，再让它们通过 production reader、preflight、lowering、geometry compile、common
admission 与 Canonical LIR；输出每模块 byte length、retained capacity、SHA-256 及完整编译
指标。正式 G3 测量仍必须由后继 evidence 子命令按冻结的 fresh-process 协议执行。
`road-editing-fixture-identities` 只审计九组合及 companion 的确定性 byte identity，不是
正式计时或峰值样本。
