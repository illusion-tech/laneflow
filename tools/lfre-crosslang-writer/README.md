# LFRE 跨语言 writer fixture

证明非 Rust writer 产出的 size-prefixed LFRE bytes 能被生产 reader
（`CompilationUnitBuilder::add_road_editing_module`）无诊断接受。合同是 **reader 接受与字段语义
等价**，不是跨语言 byte 相等——两个 golden fixture 的 vtable 布局允许不同。

- writer 源码：`cpp/writer.cpp`、`csharp/Program.cs`（+ `csharp/CrosslangWriter.csproj`）
- golden fixture：`crates/laneflow-compiler/tests/fixtures/lfre-crosslang/{cpp,csharp}_writer{,_reverse,_complete}.lfre`
- 验收测试：`crates/laneflow-compiler/tests/crosslang_lfre_fixtures.rs`

固定模块内容（两个 writer 逐字段一致）：namespace `city`、文档键 `roads/crosslang-writer`、
Direct provenance（冻结 digest 见 `crates/laneflow-compiler/src/road_editing/model.rs`）、
Fine2Cm / Smooth1Deg、一个规范坐标框架、一个走廊和两条车道。走向依次为水平长 40 米的直线、半径 35 米且水平长 45 米的圆弧，以及曲率从 1/35 降至零的 16 米回旋线。全程纵坡 40/399。
两条车道各宽 3.5 米，各有四条显式连接的边。A 在水平 47、59、68 米处切边，B 在 53、60、74 米处切边，均乘 401/399 后写为 LFRE 7 的路面站位。每种语言分别写顺向 B，以及位于独立反向区段、按 74／60／53 米保存的反向 B。Rust 读回测试将 B 的 60 米切点移到 64 米，重编并按实际规范折线核对原截面的有向边内进度。

完整横断面样例在同一走向的水平第 53 至 63.5 米开放中段，两端由相邻走廊覆盖。A 在第 59 米切边，B 仅一条边并显式声明不支承。区段内的标线、护栏和路缘在第 60 米结束。Rust 从受检制品核对局部站位、顶高和支承位。

## 钉版来源

| 来源                                       | 值                                                         | 校验                                                                                |
| ------------------------------------------ | ---------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `flatc`                                    | `v25.12.19`（设计文档钉版，见 road-editing 前端设计 §9.6） | release asset 字节数 + SHA-256（设计文档表格）                                      |
| FlatBuffers 源码（C++ 头文件 + C# 运行时） | commit `7e163021e59cca4f8e1e35a7c828b5c6b7915953`          | 源码 zip SHA-256 `bda1ae95dca76000278a05936fd8b0c66eb5c2835ccbe7aa311081faa423e482` |

C# 运行时不使用 NuGet `Google.FlatBuffers`（其上最新版落后于钉版 runtime）；直接从上述源码
`net/FlatBuffers/*.cs` 编译，与 `flatc` 保持同一 commit。

## 再生成步骤

以下命令在仓库根执行，产物全部落在 gitignore 的 `target/` 下。

1. 按设计文档钉版下载并校验 `flatc` 25.12.19（Windows asset
   `Windows.flatc.binary.zip`，SHA-256
   `fff9445c9db907227bc64b54cc98743084c4949282aa4e576cff6a955724ddc8`）。
2. 下载钉版源码 zip 并校验 SHA-256，然后提取：

   ```text
   https://codeload.github.com/google/flatbuffers/zip/7e163021e59cca4f8e1e35a7c828b5c6b7915953
   target/flatbuffers-25.12.19/include   <- include/
   target/flatbuffers-25.12.19/net       <- net/
   ```

3. 用钉版 argv 生成绑定：

   ```text
   flatc --cpp -o target/road-editing-codegen/cpp schemas/road-editing/v7/road-editing.fbs
   flatc --csharp -o target/road-editing-codegen/csharp schemas/road-editing/v7/road-editing.fbs
   ```

4. 运行两个 writer。每个写入器另以 `*_writer_reverse.lfre --reverse` 和 `*_writer_complete.lfre --complete` 分别生成反向边链和完整横断面样例。

   ```bash
   mkdir -p target/tmp
   g++ -std=c++17 -Wall -Wextra \
     -I target/flatbuffers-25.12.19/include -I target/road-editing-codegen/cpp \
     tools/lfre-crosslang-writer/cpp/writer.cpp -o target/tmp/cpp-writer
   target/tmp/cpp-writer crates/laneflow-compiler/tests/fixtures/lfre-crosslang/cpp_writer.lfre
   dotnet run --project tools/lfre-crosslang-writer/csharp --configuration Release -- \
     crates/laneflow-compiler/tests/fixtures/lfre-crosslang/csharp_writer.lfre
   ```

5. 验收：`cargo +1.100.0 run --locked -p xtask -- check-wire-audit` 与
   `cargo +1.100.0 test --locked -p laneflow-compiler --test crosslang_lfre_fixtures` 必须通过。

CI（`.github/workflows/schema-codegen.yml` 的 `crosslang-fixture` job）按同一钉版流程重新生成
六份 bytes 并与 golden fixture 逐字节比对；writer 源码、fixture 或 schema 变更都会触发。
