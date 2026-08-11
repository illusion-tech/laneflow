//! 固定 FlatBuffers C++/C# writer 到 production Rust reader 的最小跨语言 fixture。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FLATBUFFERS_SOURCE_COMMIT: &str = "7e163021e59cca4f8e1e35a7c828b5c6b7915953";
#[cfg(test)]
const OUTPUT_ROOT: &str = "target/road-editing-codegen";
const CPP_GENERATED_ROOT: &str = "target/road-editing-codegen/cpp";
const CSHARP_GENERATED_ROOT: &str = "target/road-editing-codegen/csharp";
const CROSS_LANGUAGE_OUTPUT_ROOT: &str = "target/road-editing-codegen/cross-language";
const CPP_WRITER: &str =
    "research/issue-296-road-editing-source-calibration/fixtures/cpp/write_minimal.cpp";
const CSHARP_PROJECT: &str =
    "research/issue-296-road-editing-source-calibration/fixtures/csharp/CrossLanguageWriter.csproj";

#[derive(Debug, Eq, PartialEq)]
struct Arguments {
    flatc: PathBuf,
    flatbuffers_source: PathBuf,
    cxx: PathBuf,
    dotnet: PathBuf,
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let arguments = parse_arguments(args)?;
    let repository_root =
        std::env::current_dir().map_err(|error| format!("无法解析仓库根目录: {error}"))?;
    require_flatbuffers_source(&arguments.flatbuffers_source)?;

    super::road_editing_codegen::run(&[
        "--flatc".to_owned(),
        arguments.flatc.to_string_lossy().into_owned(),
    ])?;

    let output_root = repository_root.join(CROSS_LANGUAGE_OUTPUT_ROOT);
    fs::create_dir_all(&output_root).map_err(|error| {
        format!(
            "无法创建跨语言 fixture 输出目录 `{}`: {error}",
            output_root.display()
        )
    })?;
    let cpp_output = output_root.join("cpp.lfre");
    let csharp_output = output_root.join("csharp.lfre");
    compile_and_run_cpp_writer(
        &repository_root,
        &arguments.flatbuffers_source,
        &arguments.cxx,
        &cpp_output,
    )?;
    compile_and_run_csharp_writer(
        &repository_root,
        &arguments.flatbuffers_source,
        &arguments.dotnet,
        &csharp_output,
    )?;
    validate_fixture_path(&cpp_output)?;
    validate_fixture_path(&csharp_output)?;
    println!(
        "跨语言 writer fixture 已生成：C++={}；C#={}",
        cpp_output.display(),
        csharp_output.display()
    );
    Ok(())
}

fn parse_arguments(args: &[String]) -> Result<Arguments, String> {
    match args {
        [flatc_flag, flatc, source_flag, source, cxx_flag, cxx, dotnet_flag, dotnet]
            if flatc_flag == "--flatc"
                && source_flag == "--flatbuffers-source"
                && cxx_flag == "--cxx"
                && dotnet_flag == "--dotnet"
                && [flatc, source, cxx, dotnet]
                    .iter()
                    .all(|value| !value.is_empty()) =>
        {
            Ok(Arguments {
                flatc: PathBuf::from(flatc),
                flatbuffers_source: PathBuf::from(source),
                cxx: PathBuf::from(cxx),
                dotnet: PathBuf::from(dotnet),
            })
        }
        _ => Err(
            "用法：cargo +1.96.0 run --locked -p xtask -- check-road-editing-cross-language --flatc <flatc-path> --flatbuffers-source <exact-source-checkout> --cxx <compiler> --dotnet <dotnet>"
                .to_owned(),
        ),
    }
}

fn require_flatbuffers_source(source: &Path) -> Result<(), String> {
    for relative in [
        "include/flatbuffers/flatbuffers.h",
        "net/FlatBuffers/FlatBufferBuilder.cs",
    ] {
        if !source.join(relative).is_file() {
            return Err(format!(
                "FlatBuffers source checkout 缺少 `{relative}`：`{}`",
                source.display()
            ));
        }
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("无法读取 FlatBuffers source commit: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "读取 FlatBuffers source commit 失败，exit={}",
            output.status
        ));
    }
    let commit = String::from_utf8(output.stdout)
        .map_err(|error| format!("FlatBuffers source commit 不是 UTF-8: {error}"))?;
    if commit.trim() != FLATBUFFERS_SOURCE_COMMIT {
        return Err(format!(
            "FlatBuffers source commit 不匹配：预期 `{FLATBUFFERS_SOURCE_COMMIT}`，实际 `{}`",
            commit.trim()
        ));
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(["status", "--porcelain=v1", "--untracked-files=no"])
        .output()
        .map_err(|error| format!("无法读取 FlatBuffers source 状态: {error}"))?;
    if !output.status.success() || !output.stdout.is_empty() {
        return Err("FlatBuffers source checkout 含有 tracked 修改".to_owned());
    }
    Ok(())
}

fn compile_and_run_cpp_writer(
    repository_root: &Path,
    flatbuffers_source: &Path,
    cxx: &Path,
    output: &Path,
) -> Result<(), String> {
    let binary_name = if cfg!(windows) {
        "cpp-writer.exe"
    } else {
        "cpp-writer"
    };
    let binary = repository_root
        .join(CROSS_LANGUAGE_OUTPUT_ROOT)
        .join(binary_name);
    let status = Command::new(cxx)
        .current_dir(repository_root)
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-I")
        .arg(flatbuffers_source.join("include"))
        .arg("-I")
        .arg(repository_root.join(CPP_GENERATED_ROOT))
        .arg(repository_root.join(CPP_WRITER))
        .arg("-o")
        .arg(&binary)
        .status()
        .map_err(|error| format!("无法执行 C++ compiler `{}`: {error}", cxx.display()))?;
    require_success("C++ fixture compile", status)?;
    let status = Command::new(&binary)
        .arg(output)
        .status()
        .map_err(|error| format!("无法执行 C++ fixture `{}`: {error}", binary.display()))?;
    require_success("C++ fixture writer", status)
}

fn compile_and_run_csharp_writer(
    repository_root: &Path,
    flatbuffers_source: &Path,
    dotnet: &Path,
    output: &Path,
) -> Result<(), String> {
    let build_root = repository_root
        .join(CROSS_LANGUAGE_OUTPUT_ROOT)
        .join("csharp-build");
    let status = Command::new(dotnet)
        .current_dir(repository_root)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .args(["run", "--project"])
        .arg(repository_root.join(CSHARP_PROJECT))
        .args(["--configuration", "Release"])
        .arg(format!(
            "-p:GeneratedRoot={}",
            repository_root.join(CSHARP_GENERATED_ROOT).display()
        ))
        .arg(format!(
            "-p:FlatBuffersSourceRoot={}",
            flatbuffers_source.display()
        ))
        .arg(format!(
            "-p:BaseOutputPath={}",
            msbuild_directory(&build_root.join("bin"))
        ))
        .arg(format!(
            "-p:BaseIntermediateOutputPath={}",
            msbuild_directory(&build_root.join("obj"))
        ))
        .arg("--")
        .arg(output)
        .status()
        .map_err(|error| format!("无法执行 dotnet `{}`: {error}", dotnet.display()))?;
    require_success("C# fixture writer", status)
}

fn msbuild_directory(path: &Path) -> String {
    format!("{}{}", path.display(), std::path::MAIN_SEPARATOR)
}

fn require_success(context: &str, status: std::process::ExitStatus) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("{context} 失败，exit={status}"))
    }
}

fn validate_fixture_path(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("无法读取跨语言 fixture `{}`: {error}", path.display()))?;
    validate_fixture_bytes(&bytes).map_err(|error| format!("`{}`: {error}", path.display()))
}

fn validate_fixture_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 12 {
        return Err("size-prefixed LFRE fixture 少于 12 bytes".to_owned());
    }
    let declared = u32::from_le_bytes(
        bytes[0..4]
            .try_into()
            .expect("length check proves four-byte prefix"),
    );
    let actual = u32::try_from(bytes.len() - 4)
        .map_err(|_| "fixture 长度不能表示为 u32 size prefix".to_owned())?;
    if declared != actual {
        return Err(format!(
            "size prefix 不匹配：declared={declared} actual={actual}"
        ));
    }
    if &bytes[8..12] != b"LFRE" {
        return Err("fixture 缺少固定 LFRE identifier".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_the_exact_cross_language_arguments() {
        assert_eq!(
            parse_arguments(&[
                "--flatc".to_owned(),
                "flatc".to_owned(),
                "--flatbuffers-source".to_owned(),
                "flatbuffers".to_owned(),
                "--cxx".to_owned(),
                "c++".to_owned(),
                "--dotnet".to_owned(),
                "dotnet".to_owned(),
            ]),
            Ok(Arguments {
                flatc: PathBuf::from("flatc"),
                flatbuffers_source: PathBuf::from("flatbuffers"),
                cxx: PathBuf::from("c++"),
                dotnet: PathBuf::from("dotnet"),
            })
        );
        assert!(parse_arguments(&[]).is_err());
        assert!(parse_arguments(&["--flatc".to_owned(), "flatc".to_owned()]).is_err());
    }

    #[test]
    fn requires_exact_size_prefix_and_identifier() {
        let mut valid = vec![0_u8; 12];
        valid[0..4].copy_from_slice(&8_u32.to_le_bytes());
        valid[8..12].copy_from_slice(b"LFRE");
        assert_eq!(validate_fixture_bytes(&valid), Ok(()));

        let mut wrong_size = valid.clone();
        wrong_size[0] = 7;
        assert!(validate_fixture_bytes(&wrong_size).is_err());
        let mut wrong_identifier = valid;
        wrong_identifier[8..12].copy_from_slice(b"NOPE");
        assert!(validate_fixture_bytes(&wrong_identifier).is_err());
        assert!(validate_fixture_bytes(&[0; 11]).is_err());
    }

    #[test]
    fn output_root_stays_under_the_ignored_codegen_directory() {
        assert!(CROSS_LANGUAGE_OUTPUT_ROOT.starts_with(OUTPUT_ROOT));
    }
}
