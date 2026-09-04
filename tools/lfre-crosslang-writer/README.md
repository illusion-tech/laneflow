# LFRE 跨语言 writer fixture

证明非 Rust writer 产出的 size-prefixed LFRE bytes 能被生产 reader
（`CompilationUnitBuilder::add_road_editing_module`）无诊断接受。合同是 **reader 接受与字段语义
等价**，不是跨语言 byte 相等——两个 golden fixture 的 vtable 布局允许不同。

- writer 源码：`cpp/writer.cpp`、`csharp/Program.cs`（+ `csharp/CrosslangWriter.csproj`）
- golden fixture：`crates/laneflow-compiler/tests/fixtures/lfre-crosslang/{cpp,csharp}_writer.lfre`
- 验收测试：`crates/laneflow-compiler/tests/crosslang_lfre_fixtures.rs`

固定模块内容（两个 writer 逐字段一致）：namespace `city`、文档键 `roads/crosslang-writer`、
Direct provenance（冻结 digest 见 `crates/laneflow-compiler/src/road_editing/model.rs`）、
Balanced5Cm / Balanced2Deg、一个 CanonicalFrame `frame`、一条限速 10 m/s、显式直线几何
(0,0,0)→(10,0,0) 的 LaneEdge `edge-a`，其余向量均为空。`format_version = 3`。

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
   flatc --cpp -o target/road-editing-codegen/cpp schemas/road-editing/v3/road-editing.fbs
   flatc --csharp -o target/road-editing-codegen/csharp schemas/road-editing/v3/road-editing.fbs
   ```

4. 运行两个 writer，覆盖 golden fixture：

   ```bash
   mkdir -p target/tmp
   g++ -std=c++17 -Wall -Wextra \
     -I target/flatbuffers-25.12.19/include -I target/road-editing-codegen/cpp \
     tools/lfre-crosslang-writer/cpp/writer.cpp -o target/tmp/cpp-writer
   target/tmp/cpp-writer crates/laneflow-compiler/tests/fixtures/lfre-crosslang/cpp_writer.lfre
   dotnet run --project tools/lfre-crosslang-writer/csharp --configuration Release -- \
     crates/laneflow-compiler/tests/fixtures/lfre-crosslang/csharp_writer.lfre
   ```

5. 验收：`cargo +1.98.0 run --locked -p xtask -- check-wire-audit` 与
   `cargo +1.98.0 test --locked -p laneflow-compiler --test crosslang_lfre_fixtures` 必须通过。

CI（`.github/workflows/schema-codegen.yml` 的 `crosslang-fixture` job）按同一钉版流程重新生成
两份 bytes 并与 golden fixture 逐字节比对；writer 源码、fixture 或 schema 变更都会触发。
