//! 从受检 LFRE 字节恢复可编辑的来源曲线。
use super::{
    RoadEditingCurveProgram, RoadEditingCurveSegment, RoadEditingModuleInput, RoadEditingPoint3,
};
use crate::ledger::{Ledger, LedgerStage, PassLabel, Plan};
use crate::{CompileProfile, DiagnosticBundle};
use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;

/// 曲线程序的稳定来源归属。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoadEditingCurveOwner<'a> {
    /// 道路走向的参考线。
    Alignment(&'a str),
    /// 车道边的显式几何。
    ExplicitEdge(&'a str),
}

impl RoadEditingCurveOwner<'_> {
    pub(super) fn curve(self, root: wire::RoadEditingSource<'_>) -> Option<wire::CurveProgram<'_>> {
        match self {
            Self::Alignment(key) => root
                .road_alignments()
                .iter()
                .find(|a| a.road_alignment_key() == key)
                .map(|a| a.reference_line()),
            Self::ExplicitEdge(key) => root
                .lane_edges()
                .iter()
                .find(|e| e.lane_edge_key() == key)
                .and_then(|e| e.explicit_geometry()),
        }
    }
}
fn point(p: &wire::Vec3F64) -> Result<RoadEditingPoint3, DiagnosticBundle> {
    RoadEditingPoint3::try_new(p.x(), p.y(), p.z())
}

impl RoadEditingCurveProgram {
    /// 从完整来源模块读回一条可编辑曲线，缺少所选走向或显式几何时返回空。
    ///
    /// # Errors
    ///
    /// 来源版本、校验、语义或资源准入失败时返回读取诊断。
    pub fn read_from(
        input: RoadEditingModuleInput<'_>,
        owner: RoadEditingCurveOwner<'_>,
        limits: &CompileProfile,
    ) -> Result<Option<Self>, DiagnosticBundle> {
        let ledger = Ledger::new(limits.operation_budget());
        let verified = super::reader::verify_source_in(input, limits, 0, 0, &ledger)?;
        let Some(curve) = owner.curve(verified.root()) else {
            return Ok(None);
        };
        let plan = curve.segments().iter().fold(
            Plan::ZERO.array::<RoadEditingCurveSegment>(curve.segments().len()),
            |p, s| p.array::<u8>(s.canvas_selection().map_or(0, str::len)),
        );
        let pass = ledger.admit_pass(
            PassLabel::new(LedgerStage::Admission, "road-editing-read-curve"),
            plan,
        )?;
        let grant = pass.grant_bytes(plan.bytes().unwrap_or(u64::MAX))?;
        let mut segments = Vec::with_capacity(curve.segments().len());
        for segment in curve.segments() {
            let mut editable = if let Some(line) = segment.geometry_as_line_segment() {
                RoadEditingCurveSegment::line(point(line.end())?)
            } else if let Some(cubic) = segment.geometry_as_cubic_bezier_segment() {
                RoadEditingCurveSegment::cubic_bezier(
                    point(cubic.control_1())?,
                    point(cubic.control_2())?,
                    point(cubic.end())?,
                )
            } else {
                let input = super::native_input::decode(segment)
                    .map_err(|(field, cause)| super::rules::input_error(field, cause))?;
                RoadEditingCurveSegment::native(input)?
            }
            .with_smooth_entry(segment.smooth_entry());
            if let Some(fixed) = segment.fixed_end() {
                editable = editable.with_fixed_end(point(fixed)?)?;
            }
            if let Some(canvas) = segment.canvas_selection() {
                editable = editable.with_canvas_selection(canvas)?;
            }
            segments.push(editable);
        }
        let program = Self::try_new_with_tangent(
            point(curve.start())?,
            segments,
            curve.initial_tangent_xz().map(|t| [t.x(), t.z()]),
        )?;
        drop(grant);
        pass.finish(0)?;
        Ok(Some(program))
    }
}
