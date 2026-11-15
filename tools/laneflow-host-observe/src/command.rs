//! 外部命令读取：只取标准输出，失败时给出不含输出内容的原因。

use std::{
    io,
    path::Path,
    process::{Command, Stdio},
};

/// 执行命令并返回标准输出字节；退出码非零时返回错误。错误消息只含程序名与退出
/// 状态，不含输出内容。
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
