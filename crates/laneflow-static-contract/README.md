# laneflow_static_contract

LaneFlow 编译器、线格式后发射检查与目标 Traffic Runtime 共享的静态契约值类型。

本 crate 是 `no_std`、无分配且不依赖具体编译器实现的叶子 crate。首个实现切片冻结
标识 v1（Identity v1）的实体种类 / 字段标签登记表，并提供：

- 128 位稳定标识（Stable ID）`StableId128`；
- 按实体种类区分的有类型稳定标识（Typed Stable ID）`StableId<K>`；
- 仅用于已验证致密表的有类型序号（Typed Ordinal）`Ordinal<K>`；
- 跨编译器、格式检查与运行时共享的封闭静态值（当前含 `SignalAspect`、
  `AccessEffect`）、已接受停车数值边界，以及 canonical `f32` 点、线段、长度绑定和
  连接连续性的共享空间数值边界；
- Identity v1 的版本常量、实体种类和字段标签元数据。
- 受检公历日期 `RegulationDate`、机动方向、门解释与禁令的封闭值；这些类型不执行
  运行时裁决，也不表示 LFCA/来源格式已安装对应策略表。
- LFCA 4、LFSM 3、LFSD 3 与 LFCP 2 的 magic、版本、封闭字段类型、结构安全天花板，以及
  SHA-256、路网修订标识和 exact-byte 长度值类型；
- 附录 A.1-A.4 的 section/table/field 名称、kind/tag/type/presence、singleton 行数、
  field-specific RecordVector 行和按 `u8` 判别值选择的字段存在性矩阵。

本 crate 不负责规范字节 envelope 的构造、BLAKE3 派生、来源数据解析、完整语义校验、
制品读写或运行时状态。它只提供共享的封闭值与登记形状，不建立第二套 compiler 语义后端。
