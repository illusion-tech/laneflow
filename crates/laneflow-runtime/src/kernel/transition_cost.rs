//! S4 过渡执行成本计数转发（#1233 对象 b）。
//!
//! 有 `execution-evidence` 时并入证据账本；否则为空实现，未激活的调用点
//! 不产生任何记账开销。计数与计时分离，不用时间反推次数。

#[cfg(feature = "execution-evidence")]
pub(crate) fn neighborhood(labels_scan_steps: u64, first_ring_scan_steps: u64, entries: u64) {
    crate::kernel::execution_evidence::transition_neighborhood(
        labels_scan_steps,
        first_ring_scan_steps,
        entries,
    );
}

#[cfg(not(feature = "execution-evidence"))]
pub(crate) fn neighborhood(_labels_scan_steps: u64, _first_ring_scan_steps: u64, _entries: u64) {}

#[cfg(feature = "execution-evidence")]
pub(crate) fn profile(profile_steps: u64) {
    crate::kernel::execution_evidence::transition_profile(profile_steps);
}

#[cfg(not(feature = "execution-evidence"))]
pub(crate) fn profile(_profile_steps: u64) {}
