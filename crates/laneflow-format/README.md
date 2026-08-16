# laneflow_format

LaneFlow LFCA/LFSM/LFSD/LFCP v1 的受限线格式 crate。

当前 G2 受检读取层包括：

- `ObjectPreambleV1` 与 `SectionDirectoryEntryV1` 的零拷贝 framing 预检；
- `TableV1` / `RowV1` / `FieldV1` 的冗余长度、计数、字段类型、向量、UTF-8、浮点和
  一层 `RecordVector` 通用结构预检；
- 附录 A.1-A.4 的精确 section/table/field registry、singleton 行数、由
  `sourceLocationKind/changeKind` 直接选择的线形状存在性矩阵，以及 field-specific
  RecordVector 行预检；
- 只有完整对象通过上述全部检查后才返回的 `RegistryCheckedObjectView`，以及从该能力
  零拷贝遍历 table/row/field 并按登记类型读取值的只读视图；
- 在 registry 能力之上建立的 `ValueCheckedObjectView`：检查 Identity v1 字段编码与 token、
  LFCA 的版本/封闭枚举/局部标量、LFSM 的来源种类/address/property path、LFSD 的
  Genesis/Artifact 直接绑定和同行 change 约束，以及 LFCP 的版本、receipt/publisher kind 与
  `sha256/<64 lowercase hex>` 同对象摘要绑定。

`ObjectFramingView` 只证明对象前导、目录、连续节范围和格式上限，不把节内字节暴露为
语义已验证或可信视图。`RegistryCheckedObjectView` 进一步证明附录登记形状；
`ValueCheckedObjectView` 再证明不需要外部对象或全局语义重算的直接值域与同对象绑定。两者仍不证明行排序键、
跨表引用、StableId/NetworkRevision 重算、跨对象摘要绑定、语义差异完备性或发布真实性。

本 crate 不拥有编译器语义闭包、独立语义验证、Runtime/Spatial 构造、文件系统发布事务
或信任判定。
