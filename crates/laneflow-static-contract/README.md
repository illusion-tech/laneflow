# laneflow_static_contract

LaneFlow 编译器、独立验证器与目标 Traffic Runtime 共享的静态契约值类型。

本 crate 是 `no_std`、无分配且不依赖具体编译器实现的叶子 crate。首个实现切片冻结
标识 v1（Identity v1）的实体种类 / 字段标签登记表，并提供：

- 128 位稳定标识（Stable ID）`StableId128`；
- 按实体种类区分的有类型稳定标识（Typed Stable ID）`StableId<K>`；
- 仅用于已验证致密表的有类型序号（Typed Ordinal）`Ordinal<K>`；
- Identity v1 的版本常量、实体种类和字段标签元数据。

本 crate 不负责规范字节 envelope 的构造、BLAKE3 派生、来源数据解析、语义校验、
制品读写或运行时状态。编译器与独立验证器必须各自实现身份编码与验证，避免共享实现
成为共同失效点。
