//! CLI：参数、文件读取、输出与退出码。
//!
//! 资源策略只构造一次（[`SessionConfig::tighten`]，只能收紧），inspect 与
//! diff 的全部路径共用——不存在「校验后被丢弃」的限额通道。退出码约定：
//! `0` = inspect 完成规定的单对象检查与请求的有界展示，或 diff 检查完成且
//! bytes 相同；`1` = diff 检查完成且 bytes 不同；`2` = 无法完成（参数、I/O、
//! 格式、版本、对象种类或资源限制错误）。报告正文走 stdout，执行错误走
//! stderr；诊断文本不是机器可读 schema。

use std::path::PathBuf;
use std::process::ExitCode;

use laneflow_artifact_inspect::diff::{DiffReport, diff_bytes};
use laneflow_artifact_inspect::input::{InputEncoding, InputError, read_object_input};
use laneflow_artifact_inspect::inspect::{InspectError, InspectOptions};
use laneflow_artifact_inspect::render::{render_diff, render_inspect};
use laneflow_artifact_inspect::session::{
    DisplayPolicy, InspectFilters, RowRange, SectionSelector, SessionConfig, TableSelector,
};

const USAGE: &str = "\
用法：
  laneflow-artifact-inspect inspect [选项] <PATH>
  laneflow-artifact-inspect diff [选项] <EXPECTED> <ACTUAL>

子命令：
  inspect   检视单个对象：有界摘要，可展开前导、节目录、chunk 目录与字段。
  diff      比较两份同类对象：报告原始字节、容器与字段三类差异。
            方向为 expected/old（左）→ actual/new（右）。

输入外层（默认 raw；raw 失败不自动回退 hex）：
  --hex              两个输入均按 hex 文本解码（可含 ASCII 空白）
  --hex-left         仅 expected 侧按 hex（diff）
  --hex-right        仅 actual 侧按 hex（diff）

展开（inspect）：
  --expand=LIST      逗号分隔：preamble,sections,chunks,fields

筛选（只影响展示，不改变比较结论；名称区分大小写）：
  --section=SEL      节：零基序号或登记名
  --table=SEL        表：kind 数值或登记名
  --rows=START..END  零基全表逻辑行，左闭右开

限额（只能收紧到 HARD 默认之下）：
  --max-object-bytes=N          单对象预算（默认 4294967296）
  --max-chunks-per-section=N    每 section chunk 预算（默认 65536）
  --max-report-bytes=N          报告输出字节上限（默认 65536）

其他：
  --help             本说明
  --version          工具版本与支持的对象格式版本

退出码：0 完成/相同；1 不同；2 无法完成（参数、I/O、格式、版本、对象种类
或资源限制错误）。报告正文在 stdout，执行错误在 stderr。诊断文本不是数据契约。";

const VERSION_NOTE: &str =
    "laneflow-artifact-inspect 0.0.0（对象格式：LFCA 5 / LFSM 6 / LFSD 4 / LFCP 2）";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

enum Side {
    Expected,
    Actual,
}

struct CliOptions {
    hex_left: bool,
    hex_right: bool,
    expand: laneflow_artifact_inspect::inspect::ExpandSet,
    filters: InspectFilters,
    max_object_bytes: Option<u64>,
    max_chunks_per_section: Option<u32>,
    max_report_bytes: Option<usize>,
}

impl CliOptions {
    fn new() -> Self {
        Self {
            hex_left: false,
            hex_right: false,
            expand: Default::default(),
            filters: InspectFilters::default(),
            max_object_bytes: None,
            max_chunks_per_section: None,
            max_report_bytes: None,
        }
    }

    fn encoding_for(&self, side: Side) -> InputEncoding {
        let hex = match side {
            Side::Expected => self.hex_left,
            Side::Actual => self.hex_right,
        };
        if hex {
            InputEncoding::Hex
        } else {
            InputEncoding::Raw
        }
    }

    fn session(&self) -> Result<SessionConfig, String> {
        SessionConfig::tighten(self.max_object_bytes, self.max_chunks_per_section)
            .map_err(|error| error.to_string())
    }

    fn policy(&self) -> DisplayPolicy {
        DisplayPolicy {
            filters: self.filters.clone(),
            limits: laneflow_artifact_inspect::session::ReportLimits {
                max_output_bytes: self.max_report_bytes.unwrap_or(
                    laneflow_artifact_inspect::session::ReportLimits::DEFAULT.max_output_bytes,
                ),
                ..laneflow_artifact_inspect::session::ReportLimits::DEFAULT
            },
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some((head, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    match head.as_str() {
        "--help" | "help" => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "--version" | "version" => {
            println!("{VERSION_NOTE}");
            Ok(ExitCode::SUCCESS)
        }
        "inspect" => run_inspect(rest),
        "diff" => run_diff(rest),
        other => Err(format!("未知子命令「{other}」；用 --help 查看用法")),
    }
}

fn run_inspect(args: &[String]) -> Result<ExitCode, String> {
    let mut options = CliOptions::new();
    let mut positional: Vec<PathBuf> = Vec::new();
    parse_args(args, &mut options, &mut positional, 1)?;
    let Some(path) = positional.first() else {
        return Err("inspect 需要恰好一个 PATH 参数".to_string());
    };

    let config = options.session()?;
    let policy = options.policy();
    let bytes = read_object_input(path, options.encoding_for(Side::Expected), config.input)
        .map_err(|error| input_error_message(&error, "expected"))?;

    let inspect_options = InspectOptions {
        expand: options.expand.clone(),
    };
    match laneflow_artifact_inspect::inspect::inspect(&bytes, &inspect_options, &policy, &config) {
        Ok(report) => {
            validate_filters_against_schema(&options, report.facts.kind)?;
            println!("{}", render_inspect(&report, &inspect_options, &policy));
            Ok(ExitCode::SUCCESS)
        }
        Err(InspectError::Preflight(error)) => Err(format!(
            "单对象结构与直接值域预检失败：{error:?}（偏移为结构局部坐标，不是文件绝对位置）"
        )),
        Err(error) => Err(error.to_string()),
    }
}

fn run_diff(args: &[String]) -> Result<ExitCode, String> {
    let mut options = CliOptions::new();
    let mut positional: Vec<PathBuf> = Vec::new();
    parse_args(args, &mut options, &mut positional, 2)?;
    let (Some(expected_path), Some(actual_path)) = (positional.first(), positional.get(1)) else {
        return Err("diff 需要恰好两个参数：EXPECTED ACTUAL".to_string());
    };

    let config = options.session()?;
    let policy = options.policy();
    let expected = read_object_input(
        expected_path,
        options.encoding_for(Side::Expected),
        config.input,
    )
    .map_err(|error| input_error_message(&error, "expected"))?;
    let actual = read_object_input(
        actual_path,
        options.encoding_for(Side::Actual),
        config.input,
    )
    .map_err(|error| input_error_message(&error, "actual"))?;

    if let (Ok(expected_kind), Ok(actual_kind)) = (kind_of(&expected), kind_of(&actual))
        && expected_kind != actual_kind
    {
        return Err(format!(
            "对象种类不匹配：expected 为 {expected_kind:?}，actual 为 {actual_kind:?}；diff 只比较同类对象"
        ));
    }
    if let Ok(kind) = kind_of(&expected) {
        validate_filters_against_schema(&options, kind)?;
    }

    let report: DiffReport = diff_bytes(&expected, &actual, &policy, &config);
    println!("{}", render_diff(&report, &policy));
    if report.incomplete() {
        eprintln!(
            "error: 无法完成比较：expected 检查 {}，actual 检查 {}",
            check_brief(&report.expected.check),
            check_brief(&report.actual.check)
        );
        return Ok(ExitCode::from(2));
    }
    if report.bytes_equal {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn check_brief(
    check: &Result<laneflow_static_contract::PortableObjectKind, laneflow_format::FormatError>,
) -> String {
    match check {
        Ok(kind) => format!("PASS（{kind:?}）"),
        Err(error) => format!("FAIL（{error:?}）"),
    }
}

fn kind_of(bytes: &[u8]) -> Result<laneflow_static_contract::PortableObjectKind, ()> {
    if bytes.len() < 4 {
        return Err(());
    }
    laneflow_static_contract::PortableObjectKind::from_magic(
        bytes[0..4].try_into().expect("长度已检查"),
    )
    .ok_or(())
}

fn input_error_message(error: &InputError, side: &str) -> String {
    format!("{side} 输入：{error}")
}

fn validate_filters_against_schema(
    options: &CliOptions,
    kind: laneflow_static_contract::PortableObjectKind,
) -> Result<(), String> {
    let schema = laneflow_static_contract::portable_object_schema(kind);
    if let Some(SectionSelector::Ordinal(ordinal)) = &options.filters.section
        && usize::try_from(*ordinal).map_or(true, |index| index >= schema.sections.len())
    {
        return Err(format!(
            "筛选参数错误：节序号 {ordinal} 超出范围（对象共 {} 节）",
            schema.sections.len()
        ));
    }
    if let Some(SectionSelector::Name(name)) = &options.filters.section
        && !schema.sections.iter().any(|section| section.name == *name)
    {
        return Err(format!("筛选参数错误：未知节名「{name}」"));
    }
    if let Some(TableSelector::Kind(kind_value)) = &options.filters.table
        && !schema
            .sections
            .iter()
            .flat_map(|section| section.tables.iter())
            .any(|table| table.kind == *kind_value)
    {
        return Err(format!("筛选参数错误：未知表 kind {kind_value}"));
    }
    if let Some(TableSelector::Name(name)) = &options.filters.table
        && !schema
            .sections
            .iter()
            .flat_map(|section| section.tables.iter())
            .any(|table| table.name == *name)
    {
        return Err(format!("筛选参数错误：未知表名「{name}」"));
    }
    if let Some(range) = options.filters.rows
        && range.start >= range.end
    {
        return Err(format!(
            "筛选参数错误：行范围 {}..{} 须满足 start < end",
            range.start, range.end
        ));
    }
    Ok(())
}

fn parse_args(
    args: &[String],
    options: &mut CliOptions,
    positional: &mut Vec<PathBuf>,
    expected_positional: usize,
) -> Result<(), String> {
    for arg in args {
        let Some(value) = arg.strip_prefix("--") else {
            positional.push(PathBuf::from(arg));
            continue;
        };
        let (flag, flag_value) = match value.split_once('=') {
            Some((flag, flag_value)) => (flag, Some(flag_value.to_string())),
            None => (value, None),
        };
        match flag {
            "hex" => {
                options.hex_left = true;
                options.hex_right = true;
            }
            "hex-left" => options.hex_left = true,
            "hex-right" => options.hex_right = true,
            "expand" => {
                let list = flag_value.ok_or("--expand 需要逗号分隔列表")?;
                for item in list.split(',') {
                    match item {
                        "preamble" => options.expand.preamble = true,
                        "sections" => options.expand.sections = true,
                        "chunks" => options.expand.chunks = true,
                        "fields" => options.expand.fields = true,
                        "" => {}
                        other => return Err(format!("--expand 含未知项「{other}」")),
                    }
                }
            }
            "section" => {
                let selector = flag_value.ok_or("--section 需要序号或登记名")?;
                options.filters.section = Some(parse_section_selector(&selector)?);
            }
            "table" => {
                let selector = flag_value.ok_or("--table 需要 kind 或登记名")?;
                options.filters.table = Some(parse_table_selector(&selector)?);
            }
            "rows" => {
                let range = flag_value.ok_or("--rows 需要 START..END")?;
                options.filters.rows = Some(parse_row_range(&range)?);
            }
            "max-object-bytes" => {
                let raw = flag_value.ok_or("--max-object-bytes 需要 N")?;
                options.max_object_bytes = Some(
                    raw.parse()
                        .map_err(|_| format!("--max-object-bytes 非法：{raw}"))?,
                );
            }
            "max-chunks-per-section" => {
                let raw = flag_value.ok_or("--max-chunks-per-section 需要 N")?;
                options.max_chunks_per_section = Some(
                    raw.parse()
                        .map_err(|_| format!("--max-chunks-per-section 非法：{raw}"))?,
                );
            }
            "max-report-bytes" => {
                let raw = flag_value.ok_or("--max-report-bytes 需要 N")?;
                let parsed: usize = raw
                    .parse()
                    .map_err(|_| format!("--max-report-bytes 非法：{raw}"))?;
                if parsed == 0 {
                    return Err("--max-report-bytes 必须为正".to_string());
                }
                options.max_report_bytes = Some(parsed);
            }
            other => return Err(format!("未知选项「--{other}」；用 --help 查看用法")),
        }
    }
    if positional.len() != expected_positional {
        return Err(format!(
            "位置参数数量不符：期望 {expected_positional} 个，得到 {} 个",
            positional.len()
        ));
    }
    Ok(())
}

fn parse_section_selector(raw: &str) -> Result<SectionSelector, String> {
    match raw.parse::<u32>() {
        Ok(ordinal) => Ok(SectionSelector::Ordinal(ordinal)),
        Err(_) => Ok(SectionSelector::Name(raw.to_string())),
    }
}

fn parse_table_selector(raw: &str) -> Result<TableSelector, String> {
    match raw.parse::<u16>() {
        Ok(kind) => Ok(TableSelector::Kind(kind)),
        Err(_) => Ok(TableSelector::Name(raw.to_string())),
    }
}

fn parse_row_range(raw: &str) -> Result<RowRange, String> {
    let Some((start, end)) = raw.split_once("..") else {
        return Err(format!("--rows 需要 START..END 形式，得到「{raw}」"));
    };
    let start: u32 = start
        .parse()
        .map_err(|_| format!("--rows 起始行非法：{start}"))?;
    let end: u32 = end
        .parse()
        .map_err(|_| format!("--rows 结束行非法：{end}"))?;
    if start >= end {
        return Err(format!("--rows 须满足 start < end（{start}..{end}）"));
    }
    Ok(RowRange { start, end })
}
