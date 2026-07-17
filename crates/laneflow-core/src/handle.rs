//! Core runtime typed handle。

/// lane graph edge 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EdgeHandle {
    index: u32,
}

impl EdgeHandle {
    pub(crate) fn new(index: usize) -> Self {
        Self {
            index: u32::try_from(index).expect("edge handle index must fit in u32"),
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }
}

/// immutable ParkingArea definition 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParkingAreaHandle {
    index: u32,
}

impl ParkingAreaHandle {
    pub(crate) fn new(index: usize) -> Self {
        Self {
            index: u32::try_from(index).expect("parking area handle index must fit in u32"),
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }
}

/// immutable ParkingSpace definition 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParkingSpaceHandle {
    index: u32,
}

impl ParkingSpaceHandle {
    pub(crate) fn new(index: usize) -> Self {
        Self {
            index: u32::try_from(index).expect("parking space handle index must fit in u32"),
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }
}

/// route definition 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RouteHandle {
    index: u32,
    generation: u32,
}

impl RouteHandle {
    pub(crate) fn new(index: usize, generation: u32) -> Self {
        Self {
            index: u32::try_from(index).expect("route handle index must fit in u32"),
            generation,
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }

    pub(crate) const fn generation(self) -> u32 {
        self.generation
    }
}

/// vehicle runtime entity 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleHandle {
    index: u32,
    generation: u32,
}

/// immutable Vehicle Profile 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleProfileHandle {
    index: u32,
}

/// immutable StopLine definition 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StopLineHandle {
    index: u32,
}

impl StopLineHandle {
    pub(crate) fn new(index: usize) -> Self {
        Self {
            index: u32::try_from(index).expect("stop line handle index must fit in u32"),
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }
}

/// immutable SignalGroup definition 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalGroupHandle {
    index: u32,
}

impl SignalGroupHandle {
    pub(crate) fn new(index: usize) -> Self {
        Self {
            index: u32::try_from(index).expect("signal group handle index must fit in u32"),
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }
}

/// immutable SignalController definition 的不透明 handle。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalControllerHandle {
    index: u32,
}

impl SignalControllerHandle {
    pub(crate) fn new(index: usize) -> Self {
        Self {
            index: u32::try_from(index).expect("signal controller handle index must fit in u32"),
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }
}

/// controller-local immutable phase reference。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalPhaseRef {
    controller: SignalControllerHandle,
    index: u32,
}

impl SignalPhaseRef {
    pub(crate) fn new(controller: SignalControllerHandle, index: usize) -> Self {
        Self {
            controller,
            index: u32::try_from(index).expect("signal phase index must fit in u32"),
        }
    }

    /// 返回 phase 所属 controller handle。
    pub const fn controller(self) -> SignalControllerHandle {
        self.controller
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }
}

impl VehicleProfileHandle {
    pub(crate) fn new(index: usize) -> Self {
        Self {
            index: u32::try_from(index).expect("vehicle profile handle index must fit in u32"),
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }
}

impl VehicleHandle {
    pub(crate) fn new(index: usize, generation: u32) -> Self {
        Self {
            index: u32::try_from(index).expect("vehicle handle index must fit in u32"),
            generation,
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }

    pub(crate) const fn generation(self) -> u32 {
        self.generation
    }
}
