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
