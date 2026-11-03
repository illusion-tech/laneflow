use std::path::PathBuf;

pub(super) const HELP: &str = "LaneFlow culling_extraction\n  --cells N          1..=32 (default: 9); 312 vehicles per cell\n  --camera PRESET    cell | grid | frozen (default: cell)\n  --screenshot PATH  save a .png at --at-tick and exit\n  --at-tick T        capture tick, requires --screenshot (default: 1500)\n  --help\nMouse: wheel zoom; left drag pan; right drag orbit\nF: freeze the selection volume, then zoom out to see hidden vehicles";

const DEFAULT_CELLS: usize = 9;
const MAX_CELLS: usize = 32;
const DEFAULT_CAPTURE_TICK: u64 = 1_500;
const MAX_CAPTURE_TICK: u64 = 50_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CameraPreset {
    /// 对准一格。
    #[default]
    Cell,
    /// 整张网格。
    Grid,
    /// 选择视域固定在一格，相机看整张网格，隐藏结果直接可见。
    Frozen,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CaptureRequest {
    pub path: PathBuf,
    pub tick: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RunConfig {
    pub cells: usize,
    pub camera: CameraPreset,
    pub capture: Option<CaptureRequest>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Cli {
    Help,
    Run(RunConfig),
}

pub(super) fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut args = args.into_iter();
    let mut help = false;
    let mut cells = None;
    let mut camera = None;
    let mut path = None;
    let mut tick = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" if !help => help = true,
            "--cells" if cells.is_none() => {
                let value = number(&arg, args.next())?;
                let value = usize::try_from(value).map_err(|_| "--cells 溢出".to_owned())?;
                if !(1..=MAX_CELLS).contains(&value) {
                    return Err(format!("--cells 必须在 1..={MAX_CELLS} 内"));
                }
                cells = Some(value);
            }
            "--camera" if camera.is_none() => {
                camera = Some(match args.next().as_deref() {
                    Some("cell") => CameraPreset::Cell,
                    Some("grid") => CameraPreset::Grid,
                    Some("frozen") => CameraPreset::Frozen,
                    _ => return Err("--camera 只接受 cell、grid 或 frozen".into()),
                });
            }
            "--screenshot" if path.is_none() => {
                let value = PathBuf::from(args.next().ok_or("--screenshot 缺少值")?);
                if value.extension().is_none_or(|extension| extension != "png") {
                    return Err("--screenshot 必须指向 .png 文件".into());
                }
                path = Some(value);
            }
            "--at-tick" if tick.is_none() => {
                let value = number(&arg, args.next())?;
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
    Ok(Cli::Run(RunConfig {
        cells: cells.unwrap_or(DEFAULT_CELLS),
        camera: camera.unwrap_or_default(),
        capture: path.map(|path| CaptureRequest {
            path,
            tick: tick.unwrap_or(DEFAULT_CAPTURE_TICK),
        }),
    }))
}

fn number(flag: &str, value: Option<String>) -> Result<u64, String> {
    let value = value.ok_or_else(|| format!("{flag} 缺少值"))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{flag} 必须是十进制非负整数"));
    }
    value.parse().map_err(|_| format!("{flag} 溢出"))
}
