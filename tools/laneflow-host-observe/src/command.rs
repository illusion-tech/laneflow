//! 外部命令读取：只取标准输出，失败时给出不含输出内容的原因。

use std::{
    ffi::OsString,
    io,
    path::Path,
    process::{Command, Stdio},
};

/// 去除继承的 git 环境覆盖：全部 `GIT_*` 变量（如 `GIT_DIR`、`GIT_WORK_TREE`
/// 会让读取落到另一个 checkout）与 `XDG_CONFIG_HOME`，并禁用 replace 对象，使
/// 源码状态只由调用方给定目录所在的 checkout 决定。`variables` 为当前进程的
/// 环境变量名。
pub(crate) fn sanitize_git_environment(
    command: &mut Command,
    variables: impl IntoIterator<Item = OsString>,
) {
    for key in variables {
        let text = key.to_string_lossy();
        if text.to_ascii_uppercase().starts_with("GIT_")
            || text.eq_ignore_ascii_case("XDG_CONFIG_HOME")
        {
            command.env_remove(&key);
        }
    }
    command.env("GIT_NO_REPLACE_OBJECTS", "1");
}

/// 执行命令并返回标准输出字节；退出码非零时返回错误。错误消息只含程序名与退出
/// 状态，不含输出内容。git 命令先去除继承的 git 环境覆盖。
pub(crate) fn output_bytes(
    program: &str,
    args: &[&str],
    directory: Option<&Path>,
) -> io::Result<Vec<u8>> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if program == "git" {
        sanitize_git_environment(&mut command, std::env::vars_os().map(|(key, _)| key));
    }
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let result = command.output()?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "{program} 退出状态 {}",
            result.status
        )));
    }
    Ok(result.stdout)
}

/// 执行命令并返回去除首尾空白的 UTF-8 标准输出。
pub(crate) fn output(program: &str, args: &[&str], directory: Option<&Path>) -> io::Result<String> {
    String::from_utf8(output_bytes(program, args, directory)?)
        .map(|text| text.trim().to_owned())
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{program} 输出不是 UTF-8"),
            )
        })
}
