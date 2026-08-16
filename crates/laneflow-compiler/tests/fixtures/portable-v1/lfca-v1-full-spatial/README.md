# LFCA-V1-FULL-SPATIAL fixed bytes

本目录保存 `docs/design/portable-canonical-artifact.md` 冻结的
`LFCA-V1-FULL-SPATIAL` G2 固定对象，以及同一次原子候选中的 LFSM 和 Genesis LFSD。
三份 `expected.*` 是普通、不可变的二进制 fixture；测试只通过 `include_bytes!` 读取，
不会生成、覆盖或接受新的 expected bytes。

## 规范语义输入

- 测试构造：`compiler::portable_fixture_tests::full_spatial_portable_fixture_unit`；
- authoring namespace：`city/portable-full-spatial`；
- source document：`portable-full-spatial.document`；
- source generator build ID：`git:0123456789abcdef`；
- source parameters/input digest：32 bytes `0x11`；
- source frontend-options digest：32 bytes `0x22`；
- source random seed：`42`；
- source provenance：`repository:laneflow`；
- portable compiler build ID：`laneflow-fixture-298-full-spatial-v1`；
- `FormatLimits`：`V1_HARD`；LFSD base：Genesis；
- wire format、Identity encoding/registry、NetworkRevision derivation、constraint contract、
  static execution contract、source collection digest 和 emitter version 均为 v1。

输入在一个模块中覆盖 Identity v1 全部 22 种实体。静态路线按
`entry -> middle -> exit` 穿过 `path-main`，使五张规范关系表均非空；输入同时包含
AccessRule、IIDM vehicle profile、signal controller/phases、parking、lane/facility geometry，
因此覆盖规范 i32、f64、f32、空间表和 Genesis change 投影。所有三条 LaneEdge 和
facility band 均绑定 `frame-main`，且几何长度与规范长度一致。

## 固定对象

| 文件 | exact length | SHA-256 / object key |
| --- | ---: | --- |
| `expected.lfca` | 14,649 | `sha256/87e1789dd94f664e2506c3a1f0faac1a86c647c14c3ccdafb536777d273e3a50` |
| `expected.lfsm` | 14,133 | `sha256/c3a0dd4642ef322303eaf3c7d3a3d89f4fea8da05a7f1e733538127dc8879be9` |
| `expected.lfsd` | 13,304 | `sha256/60d65447df655a68c6bda464b1dda6e9c5772fb6c674f76964e16066106543c5` |

NetworkRevisionId：
`dc1f3d54438d8ae4921dc045fd8a0c0d1a1b54363e415160445b27d427dce901`。

## 公共目录 offset 复核

三个对象的 preamble 均为 32 bytes，section directory offset 均为 32；首节 offset
分别是 LFCA `0x00e0`、LFSM `0x0098`、LFSD `0x00b0`，与设计锚点一致。

| 对象 | section kind | byte offset | byte length |
| --- | ---: | ---: | ---: |
| LFCA | 1 | 224 | 120 |
| LFCA | 2 | 344 | 7,015 |
| LFCA | 3 | 7,359 | 4,253 |
| LFCA | 4 | 11,612 | 1,238 |
| LFCA | 5 | 12,850 | 1,442 |
| LFCA | 6 | 14,292 | 64 |
| LFCA | 7 | 14,356 | 213 |
| LFCA | 8 | 14,569 | 80 |
| LFSM | 1 | 152 | 278 |
| LFSM | 2 | 430 | 4,372 |
| LFSM | 3 | 4,802 | 3,094 |
| LFSM | 4 | 7,896 | 5,526 |
| LFSM | 5 | 13,422 | 711 |
| LFSD | 1 | 176 | 293 |
| LFSD | 2 | 469 | 6,324 |
| LFSD | 3 | 6,793 | 4,884 |
| LFSD | 4 | 11,677 | 1,504 |
| LFSD | 5 | 13,181 | 20 |
| LFSD | 6 | 13,201 | 103 |

## 跨对象 binding 复核

按 v1 little-endian Table/Row/Field framing，用通用 byte reader 直接读取下列 value
offset；offset 均相对各自对象起点：

| binding | value offset | 复核结果 |
| --- | ---: | --- |
| LFCA ArtifactClaims.networkRevision | 14,617 | 等于固定 NetworkRevisionId |
| LFSM ArtifactBinding.networkRevision | 214 | 与 LFCA 相同 |
| LFSM ArtifactBinding.artifactDigest | 272 | 等于 LFCA SHA-256 |
| LFSM ArtifactBinding.artifactByteLength | 316 | `14,649` |
| LFSD DiffBinding.targetNetworkRevision | 373 | 与 LFCA 相同 |
| LFSD DiffBinding.targetArtifactDigest | 417 | 等于 LFCA SHA-256 |
| LFSD DiffBinding.targetArtifactByteLength | 461 | `14,649` |

上述 length、SHA-256、preamble、目录和 binding 复核使用 PowerShell
`Get-FileHash`、`BitConverter` 与普通 byte slices 完成，没有调用 production emitter 或
`laneflow-format` 生成期望。仓库测试另对固定 bytes 执行 `laneflow-format` 的完整结构/值域
preflight，并断言 22 张实体表、5 张关系表及两张 geometry 表均非空。

如规范语义或 wire bytes 必须变化，更新 PR 必须同时展示语义差异、版本决策、旧向量处置
和重新独立复核结果；不得把一次性 materialization 代码、JSON、Schema、xtask 或第二套
emitter 留在仓库中。
