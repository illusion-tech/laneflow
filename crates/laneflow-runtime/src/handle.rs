/// 代际感知路线句柄。只有槽位下标与 generation，不区分静态/动态。
///
/// 只在产生它的 `TrafficWorld` 内有效，不编码 world 身份，与 `VehicleHandle` /
/// ADR 0005 相同。跨 world 混用是调用方错误。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RouteHandle {
    index: u32,
    generation: u32,
}

impl RouteHandle {
    pub(crate) const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    #[must_use]
    pub(crate) const fn index(self) -> u32 {
        self.index
    }

    #[must_use]
    pub(crate) const fn generation(self) -> u32 {
        self.generation
    }
}

/// 代际感知车辆句柄。不是 Spatial `PoseRecordId`。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VehicleHandle {
    index: u32,
    generation: u32,
}

impl VehicleHandle {
    pub(crate) const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    #[must_use]
    pub(crate) const fn index(self) -> u32 {
        self.index
    }

    #[must_use]
    pub(crate) const fn generation(self) -> u32 {
        self.generation
    }
}
