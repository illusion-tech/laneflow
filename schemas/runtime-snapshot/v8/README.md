# Runtime Snapshot v8

本目录声明 LFRS 8，运行时状态版本为 6，确定性摘要版本为 9。读写、恢复和摘要的逐字段合同见[运行时快照设计 §2.1][snapshot-design]。

世界保存四项交通容量、固定步长、路权策略选择和换道驾驶配置。启用换道时，配置保存版本、跨度、反查容差和完整有序限速曲线。路线按源出现保存窗口与边界成员的稳定身份。车辆按源出现保存已提交轨迹、累计行程、交接阶段、获得刻度和仍持有的保护区间。

普通路口预约保存机动、Gate、passages 和相对于机动入口的下游终点。恢复沿路线出现前向重建物理区间，再核对整组车辆的腾让义务。车辆须保持 Waiting 遍历、成员和准入序号，以及 Conflict 的首次资格刻度、Clearing 预约和滞后历史。

发布来源绑定 asset key、LFCA 摘要、字节长度和修订。编辑来源绑定编辑状态摘要、总字节长度和模块数。恢复时由宿主提供对应的来源和执行配置。

固定 flatc 25.12.19 的生成物由以下命令核对。

```text
cargo run --locked -p xtask -- check-runtime-snapshot-codegen --flatc <flatc>
```

[snapshot-design]: ../../../docs/design/traffic-runtime-snapshot.md#21-任意位置换道与路口跟入预约的联合升版
