use std::time::{Duration, Instant};

use super::RoadEditingModuleInput;
use super::admission::{RoadEditingAdmissionObserver, RoadEditingAdmissionStage};
use crate::{CompilationUnitBuilder, DiagnosticBundle};

/// 非默认 G3 evidence feature 使用的 production admission 阶段耗时。
///
/// 本值只提供冻结的三个时间边界，不公开 verifier view、Typed AST 或其他私有阶段对象。
/// 调用方必须在 fresh process 中用完整 workload 汇总，不得把单次调试结果当成 G3 证据。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RoadEditingAdmissionStageDurations {
    size_prefix_and_identifier_preflight: Duration,
    flatbuffers_verifier: Duration,
    semantic_preflight_and_typed_ast_lowering: Duration,
}

impl RoadEditingAdmissionStageDurations {
    #[must_use]
    pub const fn size_prefix_and_identifier_preflight(self) -> Duration {
        self.size_prefix_and_identifier_preflight
    }

    #[must_use]
    pub const fn flatbuffers_verifier(self) -> Duration {
        self.flatbuffers_verifier
    }

    #[must_use]
    pub const fn semantic_preflight_and_typed_ast_lowering(self) -> Duration {
        self.semantic_preflight_and_typed_ast_lowering
    }
}

#[derive(Default)]
struct TimingObserver {
    active: Option<(RoadEditingAdmissionStage, Instant)>,
    durations: RoadEditingAdmissionStageDurations,
}

impl TimingObserver {
    fn finish(self) -> RoadEditingAdmissionStageDurations {
        debug_assert!(self.active.is_none());
        self.durations
    }
}

impl RoadEditingAdmissionObserver for TimingObserver {
    fn stage_started(&mut self, stage: RoadEditingAdmissionStage) {
        assert!(self.active.is_none(), "admission stages cannot overlap");
        self.active = Some((stage, Instant::now()));
    }

    fn stage_finished(&mut self, stage: RoadEditingAdmissionStage) {
        let (active_stage, started) = self
            .active
            .take()
            .expect("an admission stage must be active");
        assert_eq!(active_stage, stage, "admission stages must finish in order");
        let elapsed = started.elapsed();
        match stage {
            RoadEditingAdmissionStage::SizePrefixAndIdentifierPreflight => {
                self.durations.size_prefix_and_identifier_preflight = elapsed;
            }
            RoadEditingAdmissionStage::FlatbuffersVerifier => {
                self.durations.flatbuffers_verifier = elapsed;
            }
            RoadEditingAdmissionStage::SemanticPreflightAndTypedAstLowering => {
                self.durations.semantic_preflight_and_typed_ast_lowering = elapsed;
            }
        }
    }
}

impl CompilationUnitBuilder {
    /// 用 production reader/lowering 加入一个模块，并返回 G3 协议冻结的三个阶段耗时。
    ///
    /// 该入口只在非默认 `road-editing-g3-evidence` feature 下存在。它与
    /// [`CompilationUnitBuilder::add_road_editing_module`] 使用完全相同的私有准备函数与
    /// 共同原子准入；唯一差别是在三个边界调用 [`Instant::now`]。
    ///
    /// # Errors
    ///
    /// 与 [`CompilationUnitBuilder::add_road_editing_module`] 相同；任一失败均不提交候选。
    pub fn add_road_editing_module_with_stage_timing(
        &mut self,
        input: RoadEditingModuleInput<'_>,
    ) -> Result<RoadEditingAdmissionStageDurations, DiagnosticBundle> {
        let mut observer = TimingObserver::default();
        let admitted = self.prepare_road_editing_module(input, &mut observer)?;
        self.admit_official_module(admitted)?;
        Ok(observer.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_editing::{
        CanonicalFrameInput, RoadEditingDeclaration, RoadEditingModuleHeader,
        RoadEditingProvenance, RoadEditingSourceModuleBuilder, RoadEditingSourceWriter,
    };
    use crate::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile};

    #[test]
    fn stage_timing_uses_the_same_atomic_admission_path() {
        let limits = CompileLimits::p100_initial_v2();
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "roads/main",
            Vec::new(),
            RoadEditingProvenance::direct("evidence test").unwrap(),
        )
        .unwrap();
        let mut source = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .unwrap();
        source
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame").unwrap(),
            ))
            .unwrap();
        let buffer = RoadEditingSourceWriter::new(&limits)
            .write(source.finish().unwrap())
            .unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", buffer.as_bytes(), None).unwrap();
        let mut builder = CompilationUnitBuilder::new(limits);

        let durations = builder
            .add_road_editing_module_with_stage_timing(input)
            .unwrap();

        assert!(durations.size_prefix_and_identifier_preflight() > Duration::ZERO);
        assert!(durations.flatbuffers_verifier() > Duration::ZERO);
        assert!(durations.semantic_preflight_and_typed_ast_lowering() > Duration::ZERO);
        assert_eq!(builder.build().unwrap().module_count(), 1);
    }
}
