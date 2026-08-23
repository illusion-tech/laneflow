# 编译器资源与性能预算校准

**文档状态**: Retired  
**最后更新**: 2026-08-23  
**关联议题**: #308、#292

#308 是一次性非生产编译器预算校准，G4 已关闭。当前工作区不再包含研究执行器、
工作负载契约、R0 raw/Evidence JSON 或研究报告。

查阅当时制品，使用 G4 精确证据提交
[`de4cd460a96415cafbd811141568b81f74d73534`](https://github.com/illusion-tech/laneflow/tree/de4cd460a96415cafbd811141568b81f74d73534)
与交付 PR [#310](https://github.com/illusion-tech/laneflow/pull/310)
（merge `606ac52dbc75196c6d37073c72c3d48cbb031be0`）。

#292 G2 已确认这些研究工作负载不能按原自然身份无损映射为生产语义。不得把
#308 研究记录冒充产品通过（Product Pass），也不得把研究执行器加回工作区。

现行生产编译预算与 P100 基线见 `compiler-foundation.md` 与
`../reference/v0.10-compiler-production-baseline.md`。
