/// 代际感知路线句柄。静态与动态路线共用此类型；`remove_route` 必须拒绝静态句柄。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RouteHandle {
    kind: RouteKind,
    index: u32,
    generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RouteKind {
    Static,
    #[allow(dead_code)]
    Dynamic,
}

impl RouteHandle {
    pub(crate) const fn static_route(index: u32) -> Self {
        Self {
            kind: RouteKind::Static,
            index,
            generation: 0,
        }
    }

    #[allow(dead_code)]
    #[must_use]
    pub(crate) const fn is_static(self) -> bool {
        matches!(self.kind, RouteKind::Static)
    }

    #[allow(dead_code)]
    #[must_use]
    pub(crate) const fn index(self) -> u32 {
        self.index
    }

    #[allow(dead_code)]
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

#[allow(dead_code)]
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
