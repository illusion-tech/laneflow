//! `junction_debug` 命令行与环境变量回退。
use std::path::PathBuf;

/// 相机预设：`persp` 低机位透视（常规画面）；`topdown` 正交垂直向下看全路口
/// （车道几何核查用）；`close` 看路口转角；`merge` 看外围双车道的汇合/分流。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraPreset {
    Persp,
    Topdown,
    Close,
    Merge,
    Waiting,
    Signals,
}

impl CameraPreset {
    /// 未识别的名称回退到 `persp`。
    fn from_name(name: &str) -> Self {
        match name {
            "topdown" => Self::Topdown,
            "close" => Self::Close,
            "merge" => Self::Merge,
            "waiting" => Self::Waiting,
            "signals" => Self::Signals,
            _ => Self::Persp,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Persp => "persp",
            Self::Topdown => "topdown",
            Self::Close => "close",
            Self::Merge => "merge",
            Self::Waiting => "waiting",
            Self::Signals => "signals",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct CliArgs {
    pub screenshot: Option<PathBuf>,
    pub camera: CameraPreset,
    pub tick: u64,
    /// 启动即开启 F2 调试 overlay；截图模式据此出含诊断面板的图。
    pub overlay: bool,
}

impl CliArgs {
    pub(super) fn parse() -> Self {
        Self::parse_from(std::env::args().skip(1), |key| std::env::var(key).ok())
    }

    /// `--screenshot <路径> [--camera <preset>] [--at-tick <tick>] [--overlay]`；
    /// 目录写 `<目录>/junction_debug_<preset>.png`。env 回退：
    /// `JUNCTION_DEBUG_SCREENSHOT` / `JUNCTION_DEBUG_CAMERA` /
    /// `JUNCTION_DEBUG_OVERLAY`（`1`、`true` 或 `on` 开启）。
    fn parse_from(
        args: impl IntoIterator<Item = String>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let mut screenshot = None;
        let mut camera = None;
        let mut tick = 6_100;
        let mut overlay = false;
        let mut tokens = args.into_iter();
        while let Some(token) = tokens.next() {
            match token.as_str() {
                "--at-tick" => {
                    tick = tokens
                        .next()
                        .and_then(|value| value.parse().ok())
                        .expect("--at-tick requires a nonnegative integer");
                }
                "--screenshot" => screenshot = tokens.next().map(PathBuf::from),
                "--camera" => {
                    camera = Some(CameraPreset::from_name(
                        tokens.next().as_deref().unwrap_or_default(),
                    ));
                }
                "--overlay" => overlay = true,
                _ => {}
            }
        }
        if screenshot.is_none() {
            screenshot = env("JUNCTION_DEBUG_SCREENSHOT").map(PathBuf::from);
        }
        let camera = camera
            .or_else(|| env("JUNCTION_DEBUG_CAMERA").map(|name| CameraPreset::from_name(&name)))
            .unwrap_or(CameraPreset::Persp);
        overlay = overlay
            || env("JUNCTION_DEBUG_OVERLAY")
                .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on"));
        let screenshot = screenshot.map(|path| {
            let is_dir =
                path.is_dir() || path.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR);
            if is_dir {
                path.join(format!("junction_debug_{}.png", camera.tag()))
            } else {
                path
            }
        });
        Self {
            screenshot,
            camera,
            tick,
            overlay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], env: &[(&str, &str)]) -> CliArgs {
        CliArgs::parse_from(args.iter().map(|arg| (*arg).to_owned()), |key| {
            env.iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn overlay_defaults_off_and_opens_by_flag_or_environment() {
        assert!(!parse(&[], &[]).overlay);
        assert!(parse(&["--overlay"], &[]).overlay);
        assert!(parse(&[], &[("JUNCTION_DEBUG_OVERLAY", "1")]).overlay);
        assert!(!parse(&[], &[("JUNCTION_DEBUG_OVERLAY", "0")]).overlay);
    }

    #[test]
    fn flags_take_precedence_over_environment() {
        let args = parse(
            &[
                "--screenshot",
                "shot.png",
                "--camera",
                "signals",
                "--at-tick",
                "1600",
            ],
            &[
                ("JUNCTION_DEBUG_SCREENSHOT", "env.png"),
                ("JUNCTION_DEBUG_CAMERA", "close"),
            ],
        );
        assert_eq!(
            args,
            CliArgs {
                screenshot: Some(PathBuf::from("shot.png")),
                camera: CameraPreset::Signals,
                tick: 1_600,
                overlay: false,
            }
        );
        assert_eq!(
            parse(&[], &[("JUNCTION_DEBUG_CAMERA", "close")]).camera,
            CameraPreset::Close
        );
        assert_eq!(
            parse(&["--camera", "bogus"], &[]).camera,
            CameraPreset::Persp
        );
    }
}
