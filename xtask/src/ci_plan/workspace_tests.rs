//! 默认测试保留 Cargo workspace 的 feature 合并，由 nextest 选择实际运行的包。

use std::process::Command;

use super::{append_step_outputs, append_step_summary};

mod synth;

pub(super) const EXCLUDED_PACKAGES: &[&str] =
    &["laneflow-urban-harness", "laneflow-urban-generator"];
pub(super) const ALL: &str = "*";
pub(super) const EMPTY: &str = "[]";
const DEFAULT_FILTER: &str = "not (binary(corridor_traffic_smoke))";
// nextest 0.9.145 的 NO_TESTS_RUN，区别于测试、构建或过滤表达式失败。
const NO_TESTS_RUN: i32 = 4;

fn valid_package(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
}

pub(super) fn scope(full: bool, packages: &[String]) -> Result<String, String> {
    if full {
        Ok(ALL.to_owned())
    } else if packages.is_empty() {
        Ok(EMPTY.to_owned())
    } else if packages.iter().all(|name| valid_package(name)) {
        Ok(packages.join(" "))
    } else {
        Err("默认测试包名无法表示为精确的 nextest 包选择".to_owned())
    }
}

fn filterset(scope: &str) -> Result<String, String> {
    if scope == ALL {
        return Ok(DEFAULT_FILTER.to_owned());
    }
    let mut terms = Vec::new();
    for name in scope.split(' ') {
        if !valid_package(name) || EXCLUDED_PACKAGES.contains(&name) {
            return Err(format!("默认测试收到无效包名：{name:?}"));
        }
        terms.push(format!("package(={name})"));
    }
    Ok(format!("({}) & {DEFAULT_FILTER}", terms.join(" | ")))
}

fn nextest_args<'a>(filter: &'a str, targets: &'a [String]) -> Vec<&'a str> {
    let mut args = vec!["nextest", "run", "--workspace", "--locked"];
    for name in EXCLUDED_PACKAGES {
        args.extend(["--exclude", name]);
    }
    args.extend(targets.iter().map(String::as_str));
    args.extend(["-E", filter, "--no-tests", "fail"]);
    args
}

fn nextest(scope: &str) -> Result<i32, String> {
    let filter = filterset(scope)?;
    let targets = synth::cargo_targets(scope);
    if !targets.is_empty() {
        eprintln!("Synth 默认测试的 Cargo 构建目标：{}", targets.join(" "));
    }
    eprintln!("workspace 默认测试的 nextest 过滤条件：{filter}");
    Command::new("cargo")
        .args(nextest_args(&filter, &targets))
        .status()
        .map_err(|error| format!("无法运行 cargo nextest：{error}"))?
        .code()
        .ok_or_else(|| "cargo nextest 被中断".to_owned())
}

fn execute(scope: &str, mut run: impl FnMut(&str) -> Result<i32, String>) -> Result<&str, String> {
    let status = run(scope)?;
    if status == 0 {
        return Ok(scope);
    }
    if status == NO_TESTS_RUN && scope != ALL {
        eprintln!("选中的包没有可执行的默认测试，回退到完整 workspace 默认测试");
        let status = run(ALL)?;
        if status == 0 {
            return Ok(ALL);
        }
        return Err(format!(
            "全量回退的 workspace 默认测试失败，退出码 {status}"
        ));
    }
    Err(format!("workspace 默认测试失败，退出码 {status}"))
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let [scope] = args else {
        return Err("run-workspace-tests 需要一个参数（* 或以空格分隔的包名）".to_owned());
    };
    let executed = execute(scope, nextest)?;
    if executed != scope {
        append_step_summary(&format!(
            "默认测试包 `{scope}` 未选中可执行测试，已改跑完整 workspace 默认测试并通过。"
        ))?;
    }
    // 只在 nextest 成功后发布本次已覆盖的责任，包括完整默认套件覆盖所选包的情况。
    if std::env::var_os("GITHUB_OUTPUT").is_some() {
        append_step_outputs(&format!("covered_packages={scope}\n"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_target_selection_preserves_workspace_exclusions_and_test_filters() {
        let selected = filterset("laneflow-synth-harness xtask").unwrap();
        assert_eq!(
            selected,
            "(package(=laneflow-synth-harness) | package(=xtask)) & not (binary(corridor_traffic_smoke))"
        );
        let expected_prefix = [
            "nextest",
            "run",
            "--workspace",
            "--locked",
            "--exclude",
            "laneflow-urban-harness",
            "--exclude",
            "laneflow-urban-generator",
        ];
        for filter in [DEFAULT_FILTER, selected.as_str()] {
            let args = nextest_args(filter, &[]);
            assert_eq!(args[..expected_prefix.len()], expected_prefix);
            assert_eq!(
                args[expected_prefix.len()..],
                ["-E", filter, "--no-tests", "fail"]
            );
        }
        let targets = ["--lib", "--test", "cli"].map(str::to_owned);
        let args = nextest_args(&selected, &targets);
        assert_eq!(&args[..expected_prefix.len()], expected_prefix);
        assert_eq!(
            &args[expected_prefix.len()..],
            [
                "--lib",
                "--test",
                "cli",
                "-E",
                &selected,
                "--no-tests",
                "fail"
            ]
        );
        for invalid in ["", EMPTY, "a) | all()", "x\nb", "laneflow-urban-harness"] {
            assert!(filterset(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn only_a_filtered_no_tests_exit_retries_the_full_complement() {
        let mut calls = Vec::new();
        let executed = execute(synth::PACKAGE, |scope| {
            calls.push(scope.to_owned());
            Ok(if calls.len() == 1 { NO_TESTS_RUN } else { 0 })
        })
        .unwrap();
        assert_eq!(executed, ALL);
        assert_eq!(calls, [synth::PACKAGE, ALL]);
        assert!(synth::cargo_targets(ALL).is_empty());
        let mut failed_calls = 0;
        assert!(
            execute("leaf", |_| {
                failed_calls += 1;
                Ok(100)
            })
            .is_err()
        );
        assert_eq!(failed_calls, 1);
        assert!(execute("leaf", |_| Ok(NO_TESTS_RUN)).is_err());
        let mut full_calls = 0;
        assert!(
            execute(ALL, |_| {
                full_calls += 1;
                Ok(NO_TESTS_RUN)
            })
            .is_err()
        );
        assert_eq!(full_calls, 1);
    }
}
