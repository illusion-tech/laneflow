# laneflow_format

LaneFlow LFCA/LFSM/LFSD/LFCP v1 的受限线格式 crate。

当前首个 G2 切片只建立两层基础：

- `ObjectPreambleV1` 与 `SectionDirectoryEntryV1` 的零拷贝 framing 预检；
- `TableV1` / `RowV1` / `FieldV1` 的冗余长度、计数、字段类型、向量、UTF-8、浮点和
  一层 `RecordVector` 结构预检。

`ObjectFramingView` 只证明对象前导、目录、连续节范围和格式上限，不把节内字节暴露为
语义已验证或可信视图。后继切片必须在附录 A 的完整 section/table/field registry 闭合后，
才能建立对象级 typed checked view。

本 crate 不拥有编译器语义闭包、独立语义验证、Runtime/Spatial 构造、文件系统发布事务
或信任判定。
