use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Scale;
use crate::config::{CELL_SIZE_METERS, CELLS_PER_TILE};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    West,
    East,
    South,
    North,
}

impl Direction {
    pub const ALL: [Self; 4] = [Self::West, Self::East, Self::South, Self::North];

    pub const fn key(self) -> &'static str {
        match self {
            Self::West => "w",
            Self::East => "e",
            Self::South => "s",
            Self::North => "n",
        }
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::West => Self::East,
            Self::East => Self::West,
            Self::South => Self::North,
            Self::North => Self::South,
        }
    }

    pub const fn delta(self) -> (i32, i32) {
        match self {
            Self::West => (-1, 0),
            Self::East => (1, 0),
            Self::South => (0, 1),
            Self::North => (0, -1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Template {
    ProtectedWaiting,
    Permissive,
    StaggeredSouthT,
    StaggeredNorthT,
    PriorityT,
    Protected,
}

pub(crate) const TEMPLATE_ORDER: [Template; 10] = [
    Template::ProtectedWaiting,
    Template::Permissive,
    Template::StaggeredSouthT,
    Template::StaggeredNorthT,
    Template::PriorityT,
    Template::Protected,
    Template::Protected,
    Template::Protected,
    Template::Protected,
    Template::Protected,
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cell {
    pub index: u32,
    pub tile: u32,
    pub slot: u32,
    pub column: i32,
    pub row: i32,
    pub template: Template,
}

impl Cell {
    pub fn key(&self) -> String {
        format!("t{:03}.c{:02}", self.tile, self.slot)
    }

    pub fn center_meters(&self) -> (f64, f64) {
        let size = f64::from(CELL_SIZE_METERS);
        (
            f64::from(self.column) * size + size / 2.0,
            f64::from(self.row) * size + size / 2.0,
        )
    }

    pub const fn has_arm(&self, direction: Direction) -> bool {
        !matches!(
            (self.template, direction),
            (Template::StaggeredSouthT, Direction::North)
                | (
                    Template::StaggeredNorthT | Template::PriorityT,
                    Direction::South
                )
        )
    }

    pub fn edge_key(&self, direction: Direction, entering: bool) -> String {
        format!(
            "{}.{}.{}",
            self.key(),
            direction.key(),
            if entering { "in" } else { "out" }
        )
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub scale: Scale,
    pub tile_columns: u32,
    pub cells: Vec<Cell>,
    coordinates: BTreeMap<(i32, i32), usize>,
}

impl Layout {
    pub fn new(scale: Scale) -> Self {
        let mut tile_columns = 1;
        while tile_columns * tile_columns < 2 * scale.tile_count() {
            tile_columns += 1;
        }
        let mut cells = Vec::new();
        let mut coordinates = BTreeMap::new();
        for tile in 0..scale.tile_count() {
            for slot in 0..CELLS_PER_TILE {
                let column = (tile % tile_columns * 2 + slot % 2) as i32;
                let row = (tile / tile_columns * 5 + slot / 2) as i32;
                let template = TEMPLATE_ORDER[slot as usize];
                coordinates.insert((column, row), cells.len());
                cells.push(Cell {
                    index: tile * CELLS_PER_TILE + slot,
                    tile,
                    slot,
                    column,
                    row,
                    template,
                });
            }
        }
        Self {
            scale,
            tile_columns,
            cells,
            coordinates,
        }
    }

    pub fn neighbour(&self, cell: &Cell, direction: Direction) -> Option<&Cell> {
        if !cell.has_arm(direction) {
            return None;
        }
        let (dx, dz) = direction.delta();
        self.coordinates
            .get(&(cell.column + dx, cell.row + dz))
            .map(|&index| &self.cells[index])
            .filter(|other| other.has_arm(direction.opposite()))
    }
}
