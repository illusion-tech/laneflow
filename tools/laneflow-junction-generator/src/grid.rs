//! 同一正式路网中的参考路口实例；复用单路口编制与 catalog 合同。

use laneflow_scenario::complex_junction::JunctionCatalog;
use serde::{Deserialize, Serialize};

use crate::catalog::{build_catalog, validate_catalog};
use crate::compile::{compile_cells, emit_lfca};
use crate::topology::{Point, Segment, TopologyBuild, build_topology};
use crate::{Error, JunctionConfig};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GridCatalog {
    pub layout: String,
    pub columns: usize,
    pub pitch_meters: f64,
    pub cells: Vec<JunctionCatalog>,
}

/// 有界的证据输入，不定义 Runtime/Adapter 的第二套安装或求解入口。
pub struct GeneratedGrid {
    pub catalog: GridCatalog,
    pub lfca: Vec<u8>,
}

pub fn generate_grid(config: &JunctionConfig, count: usize) -> Result<GeneratedGrid, Error> {
    config.validate()?;
    if !(1..=1_000).contains(&count) {
        return Err(Error::Config("grid cell count must be 1..=1000".to_owned()));
    }
    let base = build_topology(config)?;
    let catalog = build_catalog(config, &base)?;
    validate_catalog(&catalog, &base, config)?;
    // 用全部曲线控制点的包围盒留出 100 m 隔离；Bezier 位于控制点凸包内。
    let mut radius = 0.0_f64;
    for edge in &base.edges {
        for point in std::iter::once(edge.curve.start).chain(edge.curve.segments.iter().flat_map(
            |segment| match *segment {
                Segment::Line { end } => [end, end, end],
                Segment::Bezier { c1, c2, end } => [c1, c2, end],
            },
        )) {
            radius = radius.max(point[0].abs()).max(point[1].abs());
        }
    }
    let pitch = (2.0 * radius + 100.0).ceil();
    let columns = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(columns);
    let mut cells = Vec::with_capacity(count);
    let mut catalogs = Vec::with_capacity(count);
    for index in 0..count {
        let prefix = format!("cell-{index:04}.");
        let offset = [
            ((index % columns) as f64 - (columns - 1) as f64 / 2.0) * pitch,
            ((index / columns) as f64 - (rows - 1) as f64 / 2.0) * pitch,
        ];
        let mut cell = base.clone();
        place(&mut cell, &prefix, offset);
        let mut catalog = catalog.clone();
        for route in &mut catalog.routes {
            for edge in &mut route.edge_ids {
                *edge = format!("{prefix}{edge}");
            }
        }
        for slot in &mut catalog.spawn_slots {
            slot.edge_id = format!("{prefix}{}", slot.edge_id);
        }
        // slot_id、route_id、portal_id 都在各自 catalog 内解释；物理身份来自
        // 已前缀的边与整个网络唯一的 policy/profile。原有 binder 完整复核。
        laneflow_scenario::complex_junction::validate(&catalog)
            .map_err(|error| Error::Catalog(error.to_string()))?;
        cells.push(cell);
        catalogs.push(catalog);
    }
    let compilation = compile_cells(config, &cells)?;
    Ok(GeneratedGrid {
        catalog: GridCatalog {
            layout: "junction-grid-v1".to_owned(),
            columns,
            pitch_meters: pitch,
            cells: catalogs,
        },
        lfca: emit_lfca(&compilation)?,
    })
}

fn place(topology: &mut TopologyBuild, prefix: &str, offset: Point) {
    let rename = |key: &mut String| *key = format!("{prefix}{key}");
    let translate = |point: &mut Point| {
        point[0] += offset[0];
        point[1] += offset[1];
    };
    rename(&mut topology.junction_key);
    rename(&mut topology.controller_key);
    for group in &mut topology.signal_groups {
        rename(group);
    }
    for edge in &mut topology.edges {
        rename(&mut edge.key);
        for successor in &mut edge.successors {
            rename(successor);
        }
        translate(&mut edge.curve.start);
        for segment in &mut edge.curve.segments {
            match segment {
                Segment::Line { end } => translate(end),
                Segment::Bezier { c1, c2, end } => {
                    translate(c1);
                    translate(c2);
                    translate(end);
                }
            }
        }
    }
    for path in &mut topology.paths {
        for edge in &mut path.edges {
            rename(edge);
        }
        for point in path.segment_samples.iter_mut().flatten() {
            translate(point);
        }
    }
    for stop in &mut topology.stop_lines {
        rename(&mut stop.key);
        rename(&mut stop.edge);
    }
    for gate in &mut topology.gates {
        rename(&mut gate.stop_key);
    }
    for edge in topology
        .junction_approaches
        .iter_mut()
        .chain(&mut topology.junction_internals)
    {
        rename(edge);
    }
    for zone in &mut topology.zones {
        translate(&mut zone.center);
    }
    for route in &mut topology.routes {
        for edge in &mut route.edges {
            rename(edge);
        }
    }
    for lane in topology
        .portals
        .iter_mut()
        .flat_map(|portal| &mut portal.lanes)
    {
        rename(&mut lane.edge);
    }
}
