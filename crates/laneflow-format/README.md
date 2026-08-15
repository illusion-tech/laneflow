# laneflow_format

LaneFlow LFCA/LFSM/LFSD/LFCP v1 的受限线格式 crate。

当前 G2 结构预检层包括：

- `ObjectPreambleV1` 与 `SectionDirectoryEntryV1` 的零拷贝 framing 预检；
- `TableV1` / `RowV1` / `FieldV1` 的冗余长度、计数、字段类型、向量、UTF-8、浮点和
  一层 `RecordVector` 通用结构预检；
- 附录 A.1-A.4 的精确 section/table/field registry、singleton 行数、由
  `sourceLocationKind/changeKind` 直接选择的线形状存在性矩阵，以及 field-specific
  RecordVector 行预检；
- 只有完整对象通过上述全部检查后才返回的 `RegistryCheckedObjectView`。

`ObjectFramingView` 只证明对象前导、目录、连续节范围和格式上限，不把节内字节暴露为
语义已验证或可信视图。`RegistryCheckedObjectView` 进一步证明附录登记形状，但仍不证明
行排序键、跨表引用、摘要绑定、NetworkRevision、语义差异完备性或发布真实性。

本 crate 不拥有编译器语义闭包、独立语义验证、Runtime/Spatial 构造、文件系统发布事务
或信任判定。
