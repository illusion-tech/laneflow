//! 来源无关的已发布横断面数据。
use laneflow_static_contract::{
    AuthoringLaneOrdinal, CanonicalFrameOrdinal, RoadCorridorOrdinal, RoadSectionOrdinal,
};

/// 同一条有向结构接缝的所属范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BoundaryOwner {
    /// 走廊相邻元素之间的接缝。
    Corridor(RoadCorridorOrdinal),
    /// 同一区段内相邻车道之间的接缝。
    Section(RoadSectionOrdinal),
}

/// 规则及实体长度参数均直接来自所绑定修订。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryPayload {
    /// 按有向掩码和效果解释的跨越规则。
    Rule {
        /// 1 向右、2 向左、3 双向。
        directions: u8,
        /// 1 允许、2 禁止。
        effect: u8,
    },
    /// 水平段长、半径与竖直覆盖范围以微米计。
    Guardrail {
        /// 1 锚定低站位、2 锚定高站位。
        anchor_role: u8,
        /// 相对接缝的左偏移。
        lateral_um: i32,
        /// 两端帽中心的水平距离。
        length_um: u32,
        /// 水平半径。
        radius_um: u32,
        /// 相对道路的底高与顶高。
        heights_um: [i32; 2],
        /// 参数统一误差上界，单位纳米。
        error_nm: u32,
    },
    /// 两端依次存左偏、右偏、底高和顶高。
    Curb {
        /// 低、高站位两端的长度参数，单位微米。
        ends_um: [[i32; 4]; 2],
        /// 参数统一误差上界，单位纳米。
        error_nm: u32,
    },
}

/// 按所选走廊局部站位保存的一条命名区间。
#[derive(Debug, PartialEq, Eq)]
pub struct BoundaryExtent {
    /// 所属的走廊或区段。
    pub owner: BoundaryOwner,
    /// 区间的走廊站位基准。
    pub corridor: RoadCorridorOrdinal,
    /// 在所有者有序成员表中的左成员序号。
    pub left_index: u32,
    /// 1 规则、2 实体。
    pub kind: u8,
    /// 区间的低、高端点，单位纳米。
    pub stations_nm: [u64; 2],
    /// 两端各自的站位误差上界。
    pub errors_nm: [u32; 2],
    /// 完整开放时恰有一个与种类匹配的载荷。
    pub payload: Option<BoundaryPayload>,
    pub(super) name: [u8; 53],
    pub(super) name_len: u8,
}
impl BoundaryExtent {
    /// 当前接缝内稳定的局部名。
    #[must_use]
    pub fn local_key(&self) -> &str {
        std::str::from_utf8(&self.name[..usize::from(self.name_len)]).expect("checked ASCII key")
    }
}

/// 车道的有向路幅及轮胎支承声明。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneFootprint {
    /// 编制车道序号。
    pub lane: AuthoringLaneOrdinal,
    /// 低、高站位的两条边界按左、右排列，以微米计。
    pub ends_um: [[i32; 2]; 2],
    /// 横向坐标的最大误差，单位纳米。
    pub error_nm: u32,
    /// 此路幅是否可为车轮提供支承。
    pub supports_wheel: bool,
}

/// 一段横向水平的路面。端点数据对走廊递增站位求导。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadSurfaceSpan {
    /// 本行所属走廊。
    pub corridor: RoadCorridorOrdinal,
    /// 路面使用的规范坐标系。
    pub frame: CanonicalFrameOrdinal,
    /// 两端站位及各自的误差上界，单位纳米。
    pub stations: [(u64, u32); 2],
    /// 两端各含位置、一阶和二阶导数，各三分量。
    pub jets: [[f32; 9]; 2],
    /// 零至四阶导数相对于来源的闭区间误差范数上界。
    pub derivative_errors: [f32; 5],
}
