use laneflow_scenario::signalized_corridor::CorridorPopulationConfig;

pub(super) const HELP: &str = "LaneFlow corridor_traffic\n  --vehicles N  50..=200 (default: 100)\n  --seed S      decimal u64 (default: 0)\n  --help\nMouse: wheel zoom; left drag pan; right drag orbit";

#[derive(Debug, PartialEq)]
pub(super) enum Cli {
    Help,
    Run(CorridorPopulationConfig),
}

pub(super) fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut args = args.into_iter();
    let mut vehicles = None;
    let mut seed = None;
    let mut help = false;
    while let Some(arg) = args.next() {
        if arg == "--help" && !help {
            help = true;
            continue;
        }
        let destination = match arg.as_str() {
            "--vehicles" if vehicles.is_none() => &mut vehicles,
            "--seed" if seed.is_none() => &mut seed,
            _ => return Err(format!("未知或重复参数：{arg}")),
        };
        let value = args.next().ok_or_else(|| format!("{arg} 缺少值"))?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("{arg} 必须是十进制非负整数"));
        }
        *destination = Some(value.parse::<u64>().map_err(|_| format!("{arg} 溢出"))?);
    }
    let target = usize::try_from(vehicles.unwrap_or(100)).map_err(|_| "人口溢出")?;
    let config = CorridorPopulationConfig::try_new(target, seed.unwrap_or(0))
        .map_err(|error| error.to_string())?;
    Ok(if help { Cli::Help } else { Cli::Run(config) })
}
