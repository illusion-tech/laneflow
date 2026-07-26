//! Parse DUE `local.static.*.rou.xml` vehicle/route records.

use std::str::FromStr;

use roxmltree::{Document, Node};

use crate::{
    Error, Result,
    sumo::{decimal::ExactDecimal, due::DueVehicle},
};

/// Parse one DUE routes file. `source_file_ordinal` must be 0, 1, or 2.
pub fn parse_due_routes_xml(xml: &str, source_file_ordinal: u8) -> Result<Vec<DueVehicle>> {
    if source_file_ordinal > 2 {
        return Err(Error::SumoModel(format!(
            "DUE source_file_ordinal must be 0..=2, got {source_file_ordinal}"
        )));
    }
    let document = Document::parse(xml).map_err(|source| Error::XmlParse(source.to_string()))?;
    let mut vehicles = Vec::new();
    let mut ordinal = 0_u64;
    collect_vehicles(
        document.root_element(),
        source_file_ordinal,
        &mut ordinal,
        &mut vehicles,
    )?;
    Ok(vehicles)
}

fn collect_vehicles(
    node: Node<'_, '_>,
    source_file_ordinal: u8,
    ordinal: &mut u64,
    vehicles: &mut Vec<DueVehicle>,
) -> Result<()> {
    if node.is_element() && node.tag_name().name() == "vehicle" {
        vehicles.push(parse_vehicle(node, source_file_ordinal, *ordinal)?);
        *ordinal = ordinal.checked_add(1).ok_or_else(|| {
            Error::SumoModel("DUE vehicle ordinal overflowed u64".to_owned())
        })?;
        return Ok(());
    }
    for child in node.children().filter(Node::is_element) {
        collect_vehicles(child, source_file_ordinal, ordinal, vehicles)?;
    }
    Ok(())
}

fn parse_vehicle(
    node: Node<'_, '_>,
    source_file_ordinal: u8,
    source_vehicle_ordinal: u64,
) -> Result<DueVehicle> {
    let id = required_attr(node, "id")?;
    if id.is_empty() {
        return Err(Error::SumoModel(
            "DUE vehicle id must not be empty".to_owned(),
        ));
    }
    let type_id = required_attr(node, "type")?;
    let depart = ExactDecimal::from_str(&required_attr(node, "depart")?)?;
    let route = node
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "route")
        .ok_or_else(|| {
            Error::SumoModel(format!("DUE vehicle {id:?} missing inline <route>"))
        })?;
    let edges_raw = required_attr(route, "edges")?;
    let road_edge_ids = edges_raw
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if road_edge_ids.is_empty() {
        return Err(Error::SumoModel(format!(
            "DUE vehicle {id:?} route has no edges"
        )));
    }
    Ok(DueVehicle {
        id,
        type_id,
        depart,
        road_edge_ids,
        source_file_ordinal,
        source_vehicle_ordinal,
    })
}

fn required_attr(node: Node<'_, '_>, name: &str) -> Result<String> {
    node.attribute(name).map(str::to_owned).ok_or_else(|| {
        Error::SumoModel(format!(
            "<{}> missing required attribute @{name}",
            node.tag_name().name()
        ))
    })
}
