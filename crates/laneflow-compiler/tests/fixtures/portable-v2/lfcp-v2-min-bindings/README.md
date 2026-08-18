# LFCP-V2-MIN-BINDINGS

本 fixture 固定 #299 compiler 后发射检查通过后构造的 LFCP v2 exact bytes。LFCP v2 不包含
receipt，也不绑定 LFSD；它只绑定发布所需的 LFCA、LFSM 与显式发布 provenance。

绑定输入：

- LFCA/LFSM：复用 `LFCA-V1-FULL-SPATIAL` 的固定对象；
- publisher kind：`ReleaseService (2)`；
- publisher build ID：`laneflow-publisher-fixture-v2`；
- controlled build provenance：`controlled-build`；
- controlled timestamp：`2026-08-18T00:00:00Z`。

固定结果：

- exact length：`812` bytes；
- SHA-256：`54f6ffd55c7f08f20a2f04bf273bb1b98e96ad38155f7bd027136a347ab3e763`；
- object key：
  `sha256/54f6ffd55c7f08f20a2f04bf273bb1b98e96ad38155f7bd027136a347ab3e763`；
- 第一节 offset：`0x0068`。

`expected.lfcp.hex` 是完整 LFCP exact bytes 的 lowercase hex 表示；换行只供人工复核，不属于
对象 bytes。测试只解码并比较该固定内容，不会调用 production writer 重写 expected。
