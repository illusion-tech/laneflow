"""Apply the bounded #757 prototype to an exported 46fdfaf4 tree, once."""
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
here = Path(__file__).resolve().parent

def edit(relative, old, new):
    path = root / relative
    text = path.read_text(encoding="utf-8")
    if text.count(old) != 1:
        raise RuntimeError(f"expected one anchor in {relative}: {old[:80]!r}")
    path.write_text(text.replace(old, new), encoding="utf-8", newline="\n")

runtime = "crates/laneflow-runtime/"
kernel = runtime + "src/kernel/"
harness = "tools/laneflow-urban-harness/src/"
edit(kernel + "mod.rs", "pub(crate) mod tick;", "pub(crate) mod tick;\npub(crate) mod scope;")
edit(runtime + "Cargo.toml", "placement-fixtures = []", "placement-fixtures = []\nscope-counts = []")
with (root / (runtime + "src/lib.rs")).open("a", encoding="utf-8") as f:
    f.write('\n/// #757 独立单 worker 诊断计数；本函数仅在研究补丁中存在。\n#[doc(hidden)]\npub fn research_scope_counts() -> [u64; 9] { kernel::scope::take() }\n#[doc(hidden)]\npub fn research_scope_counting_enabled() -> bool { cfg!(feature = "scope-counts") }\n')
(root / (kernel + "scope.rs")).write_bytes((here / "scope.rs").read_bytes())

edit(kernel + "tick.rs", "        let state = *self\n            .vehicle_state(vehicle)\n            .ok_or(StepError::WaitingInvariantViolation)?;", """        let state = *self
            .vehicle_state(vehicle)
            .ok_or(StepError::WaitingInvariantViolation)?;
        super::scope::count(0, 1);
        if super::scope::MODE.p2 && !self.scope_near_gate(&state, delta_s, true) {
            super::scope::count(1, 1);
            return Ok(WaitingPreviewEntry { horizon: None, preview: None });
        }""")
edit(kernel + "tick.rs", "        let preview = self\n            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, Some(horizon))", "        super::scope::count(2, 1);\n        let preview = self\n            .preview_active_vehicle_with_waiting_stop(state, delta_s, None, Some(horizon))")
edit(kernel + "tick.rs", "        #[cfg(test)]\n        MOTION_CALCULATIONS.set", "        super::scope::count(8, 1);\n        #[cfg(test)]\n        MOTION_CALCULATIONS.set")

conflict = kernel + "conflict_tick.rs"
edit(conflict, "        let projected = view\n", "        let projected = if super::scope::MODE.p3 { view.read.derived.active_order.len() } else { view\n")
edit(conflict, "            .count();\n        if inputs.try_reserve(projected)", "            .count() };\n        super::scope::count(3, if super::scope::MODE.p3 { 0 } else { view.read.committed.live_order.len() });\n        if inputs.try_reserve(projected)")
edit(conflict, "            inputs.push((vehicle, sequence, cache_index, *state));", """            super::scope::count(4, 1);
            if super::scope::MODE.p3 && !view.read.scope_near_gate(state, delta_s, false) {
                super::scope::count(5, 1);
                continue;
            }
            super::scope::count(6, 1);
            super::scope::count(7, std::mem::size_of_val(state));
            inputs.push((vehicle, sequence, cache_index, *state));""")
edit(conflict, "        let workload = inputs.len();", "        let workload = inputs.len();\n        if super::scope::MODE.p3 && workload == 0 { return Ok(true); }")
edit(conflict, "                let Some(state) = self.vehicle_state(vehicle).copied() else {", "                let Some(state) = self.vehicle_state(vehicle) else {")
edit(conflict, "                self.evaluate_vehicle_gates(\n                    state,", """                super::scope::count(4, 1);
                if super::scope::MODE.p3 && !self.read_view().scope_near_gate(state, delta_s, false) {
                    super::scope::count(5, 1);
                    continue;
                }
                let state = *state;
                super::scope::count(6, 1);
                super::scope::count(7, std::mem::size_of_val(&state));
                self.evaluate_vehicle_gates(
                    state,""")

edit(harness + "lib.rs", "mod runner;", "mod runner;\nmod scope_runner;\nmod research_quality;\npub use scope_runner::run_scope_prefix;")
edit(harness + "main.rs", "    match args.first().map(String::as_str) {", """    match args.first().map(String::as_str) {
        Some("scope-prefix") if args.len() == 6 => {
            laneflow_urban_harness::run_scope_prefix(
                Path::new(&args[1]), Path::new(&args[2]), Path::new(&args[3]),
                args[4].parse()?, args[5].parse()?,
            )?;
        }""")
for name in ["scope_runner.rs", "research_quality.rs"]:
    (root / (harness + name)).write_bytes((here / name).read_bytes())
print(f"prepared {root}")
