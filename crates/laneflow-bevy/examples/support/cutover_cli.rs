use std::path::PathBuf;

pub(super) const HELP: &str = "LaneFlow revision_cutover\n  --cutover-at-tick T  request a cutover once tick T is reached (repeatable)\n  --screenshot PATH    save a .png at --at-tick and exit\n  --at-tick T          capture tick, requires --screenshot (default: 1500)\n  --help\nKeys: C cutover\nMouse: wheel zoom; left drag pan; right drag orbit";

const DEFAULT_CAPTURE_TICK: u64 = 1_500;
const MAX_TICK: u64 = 50_000;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CaptureRequest {
    pub path: PathBuf,
    pub tick: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Cli {
    Help,
    Run {
        cutovers: Vec<u64>,
        capture: Option<CaptureRequest>,
    },
}

fn parse_tick(flag: &str, value: Option<String>) -> Result<u64, String> {
    let value = value.ok_or_else(|| format!("{flag} 缺少值"))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{flag} 必须是十进制非负整数"));
    }
    let value: u64 = value.parse().map_err(|_| format!("{flag} 溢出"))?;
    if value > MAX_TICK {
        return Err(format!("{flag} 上限为 {MAX_TICK}"));
    }
    Ok(value)
}

pub(super) fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut args = args.into_iter();
    let mut help = false;
    let mut path = None;
    let mut tick = None;
    let mut cutovers = Vec::new();
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
                tick = Some(parse_tick("--at-tick", args.next())?);
            }
            "--cutover-at-tick" => {
                cutovers.push(parse_tick("--cutover-at-tick", args.next())?);
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
    cutovers.sort_unstable();
    if cutovers.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("--cutover-at-tick 不能重复同一 Tick".into());
    }
    Ok(Cli::Run {
        cutovers,
        capture: path.map(|path| CaptureRequest {
            path,
            tick: tick.unwrap_or(DEFAULT_CAPTURE_TICK),
        }),
    })
}
