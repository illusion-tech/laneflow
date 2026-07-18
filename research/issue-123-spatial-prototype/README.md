# #123 空间几何研究原型

本目录只保存 #123 G1 使用的可执行研究原型。它作为 Rust 工作区成员接受统一测试和依赖审计，但不是生产包、公共应用程序接口（API）或已接受的数据格式；两个第三方候选只作为开发依赖（dev-dependency）使用。

原型验证：

- LaneFlow 自有的 `f64` 点、向量和位姿类型可以表达标准空间几何；
- 折线（polyline）的累计弧长以及端点、顶点处的切向量规则；
- Core 长度与几何弧长在绑定阶段的一致性检查；
- 先在 `f64` 中减去局部原点，再生成经过检查的局部 `f32` 位姿；
- 批量转换采用“全部计算成功后再提交”（compute-then-commit），从而保证失败原子性；
- `euclid`、`glam` 与 LaneFlow 自有类型的基本向量运算结果和内存布局对照。

复现命令：

```powershell
cargo +1.96.0 test -p laneflow-spatial-research --locked
cargo +stable test -p laneflow-spatial-research --locked
```

本目录中的常量和接口形状只有研究意义；正式实现必须以 #123 已接受的 ADR、设计文档和后续实施 Issue 为准。
