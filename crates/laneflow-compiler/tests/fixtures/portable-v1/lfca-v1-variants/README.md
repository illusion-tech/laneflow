# LFCA v1 固定变体向量包

本目录固定 #298 G2 的最小 headless 锚点和四类 LFCA v1 变体。测试只用
`include_bytes!` 读取这些已提交字节，再用 production compiler/emitter 生成 actual；运行测试
不会重写 expected，也没有第二套 emitter、JSON、Schema 或专用 xtask。

| 文件                      | exact length | SHA-256                                                            | NetworkRevisionId / 声明                                                   |
| ------------------------- | -----------: | ------------------------------------------------------------------ | -------------------------------------------------------------------------- |
| `min-headless.lfca`       |        1,255 | `c799e91f6a7b20d9324bccf1e6e91a12c945c0a14d914ea396211205c72d8b2b` | `4b61b28fca27bdecd0397f826cfae1ada0b2ea375b725ddc84ecd668960c1c89`         |
| `provenance-base.lfca`    |        1,251 | `eb77ff67a286a9148cc977e4a824a728ebde9269e470af7b3f7f4934b1aa8b7f` | 同最小 headless                                                            |
| `provenance-source.lfca`  |        1,251 | `67164e7fd5e50ed1c68a89dc3ed96c8d29b5a6375578ef68160c7cf0774d289d` | 同最小 headless                                                            |
| `provenance-build.lfca`   |        1,251 | `9cc40523737b85e443532042cda705f886bd916184c811d0c180079eb2a261d8` | 同最小 headless                                                            |
| `reorder-equivalent.lfca` |        3,185 | `1cb156511ca147d942875dc4a145e7477c6ce39a2132f56ef8f0603b0eb60d73` | `88605ab47eb6ecc6f0e66f5f41b704dbfcc4e16af5181cefb6c458b0d04281a2`         |
| `signed-zero.lfca`        |        2,239 | `4401ac342ee5d065694c98119d48f3ca3347ce3bb0af3f04429dc77775cc0038` | `9b1af64c32705f15ff3c7e7b4ac34b028d4fca82d83960adcd4e2b57846efaa5`         |
| `claim-mismatch.lfca`     |        1,255 | `2267ba3128b80b86623807b816516c2052212db8d218be0f6b4e0f0d8f6ee4aa` | 六个语义节仍重算为最小 headless revision；ArtifactClaims 首 byte 改为 `4a` |

语义输入和断言：

- `MIN-HEADLESS` 使用空 `CompilationUnit`，固定八节目录、首节 offset `0x00e0`、六个语义节
  length/digest、22/5/3 张表和 G1 revision 锚点。
- `PROVENANCE-ONLY` 保持六个语义节及 revision 不变，分别只改变来源文档键和显式 compiler
  build id；LFCA 与 LFSM bytes/digest 必须随对应 provenance 改变。
- `REORDER-EQUIVALENT` 使用两个互不依赖模块，每个模块含一条 root 和两条 branch。测试在
  同一已冻结来源沿袭上扰动模块集合输入、Typed AST 声明遍历和 successor 集合遍历，仍要求
  typed ordinal/reference、revision 和完整 LFCA bytes 相同。集合在 LIR 获得目标 typed
  ordinal 后原地排序；同一次 permutation 也携带 LFSM relation source，避免来源行错位。
- `SIGNED-ZERO` 把输入几何中的 `-0.0` 在编译边界规范化为 `+0.0` bits；把固定对象中该字段
  改回负零后，受检读取必须以 `NonCanonicalValue` 失败。
- `CLAIM-MISMATCH` 只翻转 `ArtifactClaims.declaredNetworkRevisionId` 的首 byte。对象结构和直接
  值域预检成功，前七节与最小对象逐字节相同，但声明值不等于从六个语义节重算的 revision。

复核命令：

```powershell
cargo +1.96.0 test --locked -p laneflow-compiler portable_ -- --nocapture
Get-ChildItem -LiteralPath crates/laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-variants -Filter *.lfca |
  Sort-Object Name |
  ForEach-Object { Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256 }
```

固定字节由 production emitter 一次性 materialize 后提交；仓库不保留写 fixture 的测试或生成
入口。任何更新都必须同时解释语义变化、旧向量处置以及 length/digest/revision 差异。
