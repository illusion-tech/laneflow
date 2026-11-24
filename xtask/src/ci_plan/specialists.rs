//! PR 验证计划的执行输出与预期步骤结果（#1464）。

use std::env;

use super::{
    Plan, append_step_outputs, append_step_summary, event_plan, plan_json_text, plan_markdown,
    plan_summary_text, workspace_tests,
};

#[derive(Debug, PartialEq, Eq)]
struct Selection {
    urban_evidence: bool,
    corridor_smoke: bool,
    placement: bool,
    urban_adapter: bool,
    evidence_clippy: bool,
    evidence_tests: bool,
    workspace_packages: String,
}

impl Selection {
    fn full() -> Self {
        Self {
            urban_evidence: true,
            corridor_smoke: true,
            placement: true,
            urban_adapter: true,
            evidence_clippy: true,
            evidence_tests: true,
            workspace_packages: workspace_tests::ALL.to_owned(),
        }
    }

    fn from_plan(plan: &Plan) -> Result<Self, String> {
        let packages = |name: &str| {
            plan.tasks
                .iter()
                .find(|task| task.name == name)
                .map(|task| task.packages.as_slice())
                .ok_or_else(|| format!("计划缺少任务 {name}"))
        };
        let selected = |name| packages(name).map(|packages| !packages.is_empty());
        let selection = Self {
            urban_evidence: selected("test-urban-evidence")?,
            corridor_smoke: selected("test-corridor-smoke")?,
            placement: selected("placement-fixtures")?,
            urban_adapter: selected("test-urban-adapter")?,
            evidence_clippy: selected("clippy-execution-evidence")?,
            evidence_tests: selected("test-execution-evidence")?,
            workspace_packages: workspace_tests::scope(plan.full, packages("test-default")?)?,
        };
        if plan.full && selection != Self::full() {
            return Err("全量计划缺少应执行的任务".to_owned());
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
            evidence_clippy,
            evidence_tests,
            workspace_packages,
        } = self;
        let outcome = |selected| if selected { "success" } else { "skipped" };
        let urban_evidence_result = outcome(*urban_evidence);
        let corridor_result = outcome(*corridor_smoke);
        let placement_result = outcome(*placement);
        let adapter_result = outcome(*urban_adapter);
        let lint_result = outcome(*evidence_clippy);
        let workspace_result = outcome(workspace_packages != workspace_tests::EMPTY);
        let tests_result = outcome(*evidence_tests);
        format!(
            "urban_evidence={urban_evidence}\n\
             corridor_smoke={corridor_smoke}\n\
             placement={placement}\n\
             urban_adapter={urban_adapter}\n\
             evidence_receipt=success/evidence/{urban_evidence_result}/{corridor_result}\n\
             placement_receipt=success/placement/{placement_result}/{placement_result}\n\
             urban_receipt=success/urban/{adapter_result}\n\
             evidence_clippy={evidence_clippy}\n\
             evidence_tests={evidence_tests}\n\
             workspace_packages={workspace_packages}\n\
             rust_receipt=success/rust/{lint_result}/{workspace_result}/{tests_result}/{workspace_packages}\n",
        )
    }
}

fn report(plan: Result<Plan, String>) -> (Selection, String) {
    let result = plan.and_then(|plan| {
        let selection = Selection::from_plan(&plan)?;
        let markdown = plan_markdown(
            "PR 验证计划（workspace 默认测试、execution-evidence、Evidence、Placement 与 Urban 按计划执行）",
            &plan_summary_text(&plan),
            &plan_json_text(&plan),
        );
        Ok((selection, markdown))
    });
    result.unwrap_or_else(|error| (
        Selection::full(),
        format!("## PR 验证计划\n\n计划生成失败，执行完整 workspace 默认测试及所有已接线专项。\n\n```text\n{error}\n```\n"),
    ))
}

pub(super) fn produce() -> Result<(), String> {
    if env::var("GITHUB_EVENT_NAME").as_deref() != Ok("pull_request") {
        return Err("ci-plan --pr-specialists 只用于 pull_request".to_owned());
    }
    let (selection, markdown) = report(event_plan("pull_request"));
    append_step_outputs(&selection.outputs())?;
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
                "evidence_clippy=true",
                "evidence_tests=false",
                "workspace_packages=[]",
                "rust_receipt=success/rust/success/skipped/skipped/[]",
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
                "evidence_clippy=false",
                "evidence_tests=false",
                "workspace_packages=[]",
                "rust_receipt=success/rust/skipped/skipped/skipped/[]",
            ]
        );
        let (full, _) = report(Ok(plan("Cargo.lock")));
        assert_eq!(full, Selection::full());

        let (leaf, _) = report(Ok(plan("tools/laneflow-synth-harness/src/main.rs")));
        assert_eq!(leaf.workspace_packages, "laneflow-synth-harness");
        assert!(leaf.outputs().contains(
            "rust_receipt=success/rust/skipped/success/skipped/laneflow-synth-harness\n"
        ));

        let mut incomplete = plan("README.md");
        incomplete
            .tasks
            .retain(|task| task.name != "test-corridor-smoke");
        let (fallback, markdown) = report(Ok(incomplete));
        assert_eq!(fallback, Selection::full());
        assert!(markdown.contains("计划缺少任务 test-corridor-smoke"));
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
                "evidence_clippy=true",
                "evidence_tests=true",
                "workspace_packages=*",
                "rust_receipt=success/rust/success/success/success/*",
            ]
        );
    }
}
