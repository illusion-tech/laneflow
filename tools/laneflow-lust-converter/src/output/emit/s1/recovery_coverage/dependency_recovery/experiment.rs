use super::*;
use combinations::Pools;

#[test]
fn quality_fit_entries_reachable_from_emit_subtree() {
    // 契约（lead E0603 同族防回归；可见性类缺陷根治）：reshape::quality 的
    // fit_with_handle/Fit 须按文档路径自 emit 兄弟子树可达（D #1154
    // calibration::witness_guided_search 调用点）；`mod quality` 可见性回归即编译
    // 失败。`fit` 保持 pub(super)（签名含 reshape 内部 Budget，不对外泄露——D 只经
    // fit_with_handle 调用）。
    use crate::output::geom::s1::reshape::quality;
    type FitWithHandle = fn(
        &[[[f64; 3]; 6]],
        &[[f64; 3]],
        &mut crate::output::geom::s1::grid::WorkBudget,
    ) -> Option<quality::Fit>;
    let _: FitWithHandle = quality::fit_with_handle;
}

#[test]
fn reference_capture_reachable_from_emit_subtree() {
    // 契约（可见性类防回归）：reshape::reference_with_curve_cap（全边参考入口，
    // D #1154 P2-1 修法）须自 emit 兄弟子树可达；可见性回归即编译失败。
    type ReferenceCapture = fn(&observe::EdgeInput, usize) -> Vec<[[f64; 3]; 6]>;
    let _: ReferenceCapture = crate::output::geom::s1::reshape::reference_with_curve_cap;
}

pub(super) fn pool(rows: &Value) -> Pools {
    rows.as_object()
        .unwrap()
        .iter()
        .filter_map(|(id, row)| {
            let candidates = row["candidates"].as_array().unwrap();
            (!candidates.is_empty()).then(|| {
                (
                    id.clone(),
                    candidates
                        .iter()
                        .map(|c| successors::frozen_candidate(c).repaired.program)
                        .collect(),
                )
            })
        })
        .collect()
}
