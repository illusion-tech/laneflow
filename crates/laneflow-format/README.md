# laneflow_format

LaneFlow 可移植规范制品 `LFCA`、源映射封套 `LFSM`、语义差异封套 `LFSD` 与规范发布描述符
`LFCP` 的受限线格式 crate。LFCA/LFSM/LFSD 对象版本只承认当前组合 `5/4/4`。规范术语见
[`docs/reference/glossary.md`](../../docs/reference/glossary.md)。

当前线格式层包括：

- 无分配的 `measure_object` / `prepare_object` / `encode_object` 受限精确编码：接收借用的
  对象、节、表、行和字段输入，复用附录 A registry，先完成
  全对象计量、形状和预算检查；`PreparedObject` 可把这一次预检和 exact length 带入
  `encode_prepared_object`，避免精确分配后重复预检；只有调用方缓冲区长度精确匹配后
  才写入，任何返回错误都保持输出逐字节不变；

- 对象前导与节目录项的零拷贝 framing 预检；
- 表 / 行 / 字段的冗余长度、计数、字段类型、向量、UTF-8、浮点和
  一层 `RecordVector` 通用结构预检；
- 附录 A.1-A.4 的精确 section/table/field registry、singleton 行数、由
  `sourceLocationKind/changeKind` 直接选择的线形状存在性矩阵，以及 field-specific
  RecordVector 行预检；
- 只有完整对象通过上述全部检查后才返回的 `RegistryCheckedObjectView`，以及从该能力
  以单游标零拷贝遍历 section/table/row/field 并按登记类型读取值的只读视图；保留的
  ordinal 随机访问 API 为 O(n)，不用于顺序扫描；
- 在 registry 能力之上建立的 `ValueCheckedObjectView`：检查 Identity v1 字段编码与 token、
  LFCA 的版本/封闭枚举/实体内部向量基数/局部标量、LFSM 的来源种类/address/property path、
  LFSD 的 Genesis/Artifact 直接绑定和同行 change 约束，以及 LFCP v2 的版本、publisher kind 与
  `sha256/<64 lowercase hex>` 同对象摘要绑定。
- LFSD 4 的第七节及四类完整成员 RowV1：Bytes 内部也执行字段/字符串/向量计量，
  两侧载荷共同收费并参与规范分块；检查稳定引用编码、成员 K 的跨 chunk 顺序和全局
  唯一性。真实两根的策略差异完备性由 compiler 的独立检查入口证明。
- 无分配的 `check_post_emission_bundle`：从三份最终字节重算 digest、length 与 LFCA
  `NetworkRevisionId`，并闭合 LFSM provenance/artifact binding 以及 LFSD 显式 base/target
  binding；成功后只返回字段私有的借用型发布能力。它不重跑完整路网语义或验证 LFSD
  change set 完备性。
- `check_canonical_network_input` 与 `PostEmissionCheckedBundle::canonical_network_input`：
  单份已通过宿主 admission 的 LFCA 和同进程 compiler bundle 共用同一份
  digest/length/`NetworkRevisionId` 检查与字段私有 `CheckedCanonicalNetworkInput`
  能力；该能力是 `laneflow-static-network` 的唯一构建输入，但不表示发布真实性或
  Runtime 跨表闭合已经成立。
- `FormatLimits` 同时覆盖对象/节/表、行/字段、Identity ASCII、UTF-8、向量、嵌套、LFSM
  来源位置与候选暂存预算；任一调用方值只能收紧。registry capability 保留产生它的同一
  limits，后续直接值域检查不能通过换回较大 limits 绕过调用方预算。

`ObjectFramingView` 只证明对象前导、目录、连续节范围和格式上限，不把节内字节暴露为
语义已验证或可信视图。`RegistryCheckedObjectView` 进一步证明附录登记形状；
`ValueCheckedObjectView` 再证明不需要外部对象或全局语义重算的直接值域、策略局部行排序与同对象绑定。两者仍不证明其他通用行排序键、
跨表引用、StableId/NetworkRevision 重算、跨对象摘要绑定、语义差异完备性或发布真实性。
只有额外取得 `CheckedCanonicalNetworkInput` 后，LFCA 的实际摘要、精确长度和重算
`NetworkRevisionId` 才被绑定；跨表、Identity 双射与 Traffic/Spatial 闭合继续由共享静态
路网构建器负责。

写入输入只表达线格式值，不是 compiler LIR 或 `ValueCheckedObjectView`。writer 保证输出满足
framing、registry、冗余长度/计数、UTF-8/向量预算和通用浮点位模式约束；field-specific
直接值域、跨表闭包和跨对象绑定仍由 compiler emitter 的已验证输入与后继验证层负责。

本 crate 不拥有编译器完整语义闭包、Runtime/Spatial 构造、文件系统发布事务
或信任判定。
