//! PR 的专项执行选择与预期步骤结果（#1464）。

use std::env;
use std::fs::OpenOptions;
use std::io::Write;

use super::{
    Plan, append_step_summary, event_plan, plan_json_text, plan_markdown, plan_summary_text,
};

#[derive(Debug, PartialEq, Eq)]
struct Selection {
    urban_evidence: bool,
    corridor_smoke: bool,
    placement: bool,
    urban_adapter: bool,
}

impl Selection {
    const FULL: Self = Self {
        urban_evidence: true,
        corridor_smoke: true,
        placement: true,
        urban_adapter: true,
    };

    fn from_plan(plan: &Plan) -> Result<Self, String> {
        let selected = |name| {
            plan.tasks
                .iter()
                .find(|task| task.name == name)
                .map(|task| !task.packages.is_empty())
                .ok_or_else(|| format!("计划缺少专项任务 {name}"))
        };
        let selection = Self {
            urban_evidence: selected("test-urban-evidence")?,
            corridor_smoke: selected("test-corridor-smoke")?,
            placement: selected("placement-fixtures")?,
            urban_adapter: selected("test-urban-adapter")?,
        };
        if plan.full && selection != Self::FULL {
            return Err("全量计划缺少应执行的专项".to_owned());
        }
        Ok(selection)
    }

    /// 每个 job 用生产计划的 job.result、自己的名称与实际 steps.*.outcome 对账。
    /// 即使全部步骤应跳过，也必须核对到成功的计划和每个步骤显式的 skipped。
    fn outputs(&self) -> String {
        let Self {
            urban_evidence,
            corridor_smoke,
            placement,
            urban_adapter,
        } = self;
        let outcome = |selected| if selected { "success" } else { "skipped" };
        let urban_evidence_result = outcome(*urban_evidence);
        let corridor_result = outcome(*corridor_smoke);
        let placement_result = outcome(*placement);
        let adapter_result = outcome(*urban_adapter);
        format!(
            "urban_evidence={urban_evidence}\n\
             corridor_smoke={corridor_smoke}\n\
             placement={placement}\n\
             urban_adapter={urban_adapter}\n\
             evidence_receipt=success/evidence/{urban_evidence_result}/{corridor_result}\n\
             placement_receipt=success/placement/{placement_result}/{placement_result}\n\
             urban_receipt=success/urban/{adapter_result}\n",
        )
    }
}

fn report(plan: Result<Plan, String>) -> (Selection, String) {
    let result = plan.and_then(|plan| {
        let selection = Selection::from_plan(&plan)?;
        let markdown = plan_markdown(
            "PR 验证计划（Evidence、Placement、Urban 按计划执行，其余任务作影子对照）",
            &plan_summary_text(&plan),
            &plan_json_text(&plan),
        );
        Ok((selection, markdown))
    });
    result.unwrap_or_else(|error| (
        Selection::FULL,
        format!("## PR 专项验证计划\n\n计划生成失败，Evidence、Placement、Urban 执行全部专项。\n\n```text\n{error}\n```\n"),
    ))
}

pub(super) fn produce() -> Result<(), String> {
    if env::var("GITHUB_EVENT_NAME").as_deref() != Ok("pull_request") {
        return Err("ci-plan --pr-specialists 只用于 pull_request".to_owned());
    }
    let (selection, markdown) = report(event_plan("pull_request"));
    let path = env::var_os("GITHUB_OUTPUT").ok_or("缺少专项计划输出 GITHUB_OUTPUT")?;
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| format!("无法打开 GITHUB_OUTPUT：{error}"))?;
    file.write_all(selection.outputs().as_bytes())
        .map_err(|error| format!("无法写入专项计划：{error}"))?;
    append_step_summary(&markdown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ci_plan::{
        REGISTRY_TOML, Registry, RegistryCheck, Workspace, build_plan, cargo_metadata,
    };

    #[test]
    fn pr_outputs_match_the_selected_steps_and_retain_the_other_shadow_tasks() {
        let metadata = cargo_metadata().unwrap();
        let workspace = Workspace::parse(&metadata).unwrap();
        let registry = Registry::parse(REGISTRY_TOML).unwrap();
        let plan = |path: &str| {
            build_plan(
                &[path.to_owned()].into(),
                &workspace,
                &registry,
                RegistryCheck::Consistent {
                    cross_package_edges: 0,
                },
            )
        };
        let (selection, markdown) = report(Ok(plan("tools/laneflow-urban-harness/src/main.rs")));
        assert_eq!(
            selection.outputs().lines().collect::<Vec<_>>(),
            [
                "urban_evidence=true",
                "corridor_smoke=false",
                "placement=false",
                "urban_adapter=true",
                "evidence_receipt=success/evidence/success/skipped",
                "placement_receipt=success/placement/skipped/skipped",
                "urban_receipt=success/urban/success",
            ]
        );
        assert!(markdown.contains("\"name\": \"test-default\""));
        assert!(markdown.contains("\"name\": \"wire-audit\""));
        assert!(markdown.contains("tools/laneflow-urban-harness/src/main.rs"));

        let (empty, _) = report(Ok(plan("README.md")));
        assert_eq!(
            empty.outputs().lines().skip(4).collect::<Vec<_>>(),
            [
                "evidence_receipt=success/evidence/skipped/skipped",
                "placement_receipt=success/placement/skipped/skipped",
                "urban_receipt=success/urban/skipped",
            ]
        );
        let (full, _) = report(Ok(plan("Cargo.lock")));
        assert_eq!(full, Selection::FULL);

        let mut incomplete = plan("README.md");
        incomplete
            .tasks
            .retain(|task| task.name != "test-corridor-smoke");
        let (fallback, markdown) = report(Ok(incomplete));
        assert_eq!(fallback, Selection::FULL);
        assert!(markdown.contains("计划缺少专项任务 test-corridor-smoke"));
    }

    #[test]
    fn failed_planning_requires_success_for_every_specialist_step() {
        let (fallback, markdown) = report(Err("无法读取完整 diff".to_owned()));
        assert!(markdown.contains("无法读取完整 diff"));
        assert_eq!(
            fallback.outputs().lines().collect::<Vec<_>>(),
            [
                "urban_evidence=true",
                "corridor_smoke=true",
                "placement=true",
                "urban_adapter=true",
                "evidence_receipt=success/evidence/success/success",
                "placement_receipt=success/placement/success/success",
                "urban_receipt=success/urban/success",
            ]
        );
    }
}
