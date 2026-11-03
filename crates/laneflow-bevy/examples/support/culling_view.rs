//! 不依赖渲染器的宿主视域与车道分块；GUI 与 headless 走同一判定。
use bevy_ecs::resource::Resource;
use bevy_math::{Mat4, Vec3, Vec4};
use laneflow_static_contract::LaneEdgeOrdinal;

const CHUNK_MM: u32 = 25_000;

/// 平面集合：`normal.dot(p) + d >= 0` 视为在内侧。
#[derive(Resource, Clone, Debug, PartialEq)]
pub(super) struct ViewVolume {
    planes: [Vec4; 6],
}

impl ViewVolume {
    /// 不裁剪任何位置。
    pub const ALL: Self = Self {
        planes: [Vec4::W; 6],
    };

    /// 从 Bevy 约定的裁剪矩阵（x、y 在 `-w..=w`，z 在 `0..=w`）提取平面。
    /// 无限远反向投影的远平面退化为恒真，不需要特殊处理。
    pub fn from_clip_from_world(clip_from_world: Mat4) -> Self {
        let [x, y, z, w] = [0, 1, 2, 3].map(|row| clip_from_world.row(row));
        Self {
            planes: [w + x, w - x, w + y, w - y, z, w - z],
        }
    }

    /// 保守判定：包围盒完全位于某个平面外侧才不相交。
    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|plane| {
            let normal = plane.truncate();
            let farthest = Vec3::select(normal.cmpge(Vec3::ZERO), max, min);
            normal.dot(farthest) + plane.w >= 0.0
        })
    }
}

/// 每条车道按固定弧长分块的预计算包围盒，已扩张车身余量。
#[derive(Resource, Debug)]
pub(super) struct LaneChunks {
    first_chunk: Vec<u32>,
    bounds: Vec<(Vec3, Vec3)>,
    visible: Vec<bool>,
}

impl LaneChunks {
    pub fn build(lanes: &[Vec<Vec3>], lengths_mm: &[u32], margin: f32) -> Result<Self, String> {
        if lanes.len() != lengths_mm.len() {
            return Err("车道几何与长度数量不一致".into());
        }
        let mut first_chunk = Vec::with_capacity(lanes.len() + 1);
        let mut bounds = Vec::new();
        for (points, &length_mm) in lanes.iter().zip(lengths_mm) {
            first_chunk.push(u32::try_from(bounds.len()).map_err(|_| "分块数量溢出")?);
            push_lane_chunks(points, length_mm, margin, &mut bounds)?;
        }
        first_chunk.push(u32::try_from(bounds.len()).map_err(|_| "分块数量溢出")?);
        let visible = vec![false; bounds.len()];
        Ok(Self {
            first_chunk,
            bounds,
            visible,
        })
    }

    pub fn refresh(&mut self, view: &ViewVolume) {
        for (visible, (min, max)) in self.visible.iter_mut().zip(&self.bounds) {
            *visible = view.intersects(*min, *max);
        }
    }

    /// 车道外的来源返回 `None`，由调用方按失败处理。
    pub fn is_visible(&self, edge: LaneEdgeOrdinal, progress_mm: u32) -> Option<bool> {
        let index = edge.index();
        let first = *self.first_chunk.get(index)? as usize;
        let end = *self.first_chunk.get(index + 1)? as usize;
        let offset = (progress_mm / CHUNK_MM) as usize;
        let chunk = (first + offset).min(end.checked_sub(1)?);
        self.visible.get(chunk).copied()
    }
}

fn push_lane_chunks(
    points: &[Vec3],
    length_mm: u32,
    margin: f32,
    bounds: &mut Vec<(Vec3, Vec3)>,
) -> Result<(), String> {
    if points.len() < 2 || length_mm == 0 {
        return Err("无效车道 centerline".into());
    }
    let polyline: f32 = points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum();
    if !polyline.is_finite() || polyline <= 0.0 {
        return Err("无效车道 centerline".into());
    }
    // 分块边界按交通长度比例映射到折线弧长；舍入差由余量覆盖。
    let count = length_mm.div_ceil(CHUNK_MM);
    let chunk_arc = polyline * (CHUNK_MM as f32 / length_mm as f32);
    let mut min = points[0];
    let mut max = points[0];
    let mut chunk = 1;
    let mut travelled = 0.0;
    for pair in points.windows(2) {
        let segment = pair[0].distance(pair[1]);
        while chunk < count && travelled + segment >= chunk_arc * chunk as f32 {
            let cut = pair[0].lerp(pair[1], (chunk_arc * chunk as f32 - travelled) / segment);
            bounds.push((min.min(cut) - margin, max.max(cut) + margin));
            min = cut;
            max = cut;
            chunk += 1;
        }
        min = min.min(pair[1]);
        max = max.max(pair[1]);
        travelled += segment;
    }
    bounds.push((min - margin, max + margin));
    // 浮点累计可能少切一块；补齐后 progress 到分块的映射仍然保守。
    while chunk < count {
        let last = bounds[bounds.len() - 1];
        bounds.push(last);
        chunk += 1;
    }
    Ok(())
}
