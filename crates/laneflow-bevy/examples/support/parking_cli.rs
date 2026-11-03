use std::path::PathBuf;

pub(super) const HELP: &str = "LaneFlow parking_lifecycle\n  --screenshot PATH  save a .png at --at-tick and exit\n  --at-tick T        capture tick, requires --screenshot (default: 1500)\n  --help\nMouse: wheel zoom; left drag pan; right drag orbit";

const DEFAULT_CAPTURE_TICK: u64 = 1_500;
const MAX_CAPTURE_TICK: u64 = 50_000;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CaptureRequest {
    pub path: PathBuf,
    pub tick: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Cli {
    Help,
    Run { capture: Option<CaptureRequest> },
}

pub(super) fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut args = args.into_iter();
    let mut help = false;
    let mut path = None;
    let mut tick = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" if !help => help = true,
            "--screenshot" if path.is_none() => {
                let value = PathBuf::from(args.next().ok_or("--screenshot 缺少值")?);
                if value.extension().is_none_or(|extension| extension != "png") {
                    return Err("--screenshot 必须指向 .png 文件".into());
                }
                path = Some(value);
            }
            "--at-tick" if tick.is_none() => {
                let value = args.next().ok_or("--at-tick 缺少值")?;
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err("--at-tick 必须是十进制非负整数".into());
                }
                let value: u64 = value.parse().map_err(|_| "--at-tick 溢出".to_owned())?;
                if value > MAX_CAPTURE_TICK {
                    return Err(format!("--at-tick 上限为 {MAX_CAPTURE_TICK}"));
                }
                tick = Some(value);
            }
            _ => return Err(format!("未知或重复参数：{arg}")),
        }
    }
    if tick.is_some() && path.is_none() {
        return Err("--at-tick 需要 --screenshot".into());
    }
    if help {
        return Ok(Cli::Help);
    }
    Ok(Cli::Run {
        capture: path.map(|path| CaptureRequest {
            path,
            tick: tick.unwrap_or(DEFAULT_CAPTURE_TICK),
        }),
    })
}
