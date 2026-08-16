# LFCP-V1-MIN-BINDINGS

本 fixture 固定 #298 拥有的 LFCP 外部绑定与 exact bytes，不冻结 #299 尚未设计的 validation
receipt wire。测试中的 receipt 只是以下 55-byte opaque stand-in；它只用于固定 LFCP 中的
digest、exact length、validator metadata 与 content-addressed key：

```text
test-only opaque #299 receipt bytes for LFCP binding v1
```

绑定输入：

- LFCA/LFSM：复用 `LFCA-V1-FULL-SPATIAL` 的固定对象；
- LFCA object key：
  `sha256/87e1789dd94f664e2506c3a1f0faac1a86c647c14c3ccdafb536777d273e3a50`；
- LFSM object key：
  `sha256/c3a0dd4642ef322303eaf3c7d3a3d89f4fea8da05a7f1e733538127dc8879be9`；
- receipt format version：`1`；
- receipt kind：`canonical-publication-v1`；
- validator build ID：`laneflow-validator-fixture-v1`；
- receipt object key：
  `sha256/8b06421c8600c603c3c89f97b0ef3ffd76c5740552f6e0888b5737f818a1c738`；
- publisher kind：`LocalTool (0)`；
- publisher build ID：`laneflow-publisher-fixture-v1`；
- optional controlled build provenance/timestamp：均缺失。

固定结果：

- exact length：`1,050` bytes；
- SHA-256：`7cbe21a42bca1f50f30e34de91db310e8d550e64f87a761cd1bec516010c4e05`；
- object key：
  `sha256/7cbe21a42bca1f50f30e34de91db310e8d550e64f87a761cd1bec516010c4e05`；
- 第一节 offset：`0x0080`。

`expected.lfcp.hex` 是完整 LFCP exact bytes 的 lowercase hex 表示；换行只供人工复核，不属于
对象 bytes。测试只解码并比较该固定内容，不会调用 production writer 重写 expected。
