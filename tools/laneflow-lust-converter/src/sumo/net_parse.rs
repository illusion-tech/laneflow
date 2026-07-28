//! Parse SUMO `lust.net.xml` (or fixture nets) into [`SumoNetwork`].

use std::str::FromStr;

use roxmltree::{Document, Node};

use crate::{
    Error, Result,
    sumo::{
        decimal::ExactDecimal,
        net::{
            SumoConnection, SumoEdge, SumoJunction, SumoLane, SumoLocation, SumoNetwork, SumoTlLogic,
            SumoTlPhase,
        },
    },
};

/// Parse a SUMO network XML document.
pub fn parse_sumo_network_xml(xml: &str) -> Result<SumoNetwork> {
    let document = Document::parse(xml).map_err(|source| Error::XmlParse(source.to_string()))?;
    let root = document.root_element();
    if root.tag_name().name() != "net" {
        return Err(Error::SumoModel(format!(
            "expected root <net>, found <{}>",
            root.tag_name().name()
        )));
    }

    let location = parse_location(root)?;
    let mut edges = Vec::new();
    let mut lanes = Vec::new();
    let mut junctions = Vec::new();
    let mut connections = Vec::new();
    let mut tl_logics = Vec::new();

    for child in root.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "edge" => parse_edge(child, &mut edges, &mut lanes)?,
            "junction" => junctions.push(parse_junction(child)?),
            "connection" => connections.push(parse_connection(child)?),
            "tlLogic" => tl_logics.push(parse_tl_logic(child)?),
            _ => {}
        }
    }

    if lanes.is_empty() {
        return Err(Error::SumoModel(
            "SUMO network contains no <lane> elements".to_owned(),
        ));
    }

    Ok(SumoNetwork {
        location,
        edges,
        lanes,
        junctions,
        connections,
        tl_logics,
    })
}

fn parse_location(root: Node<'_, '_>) -> Result<SumoLocation> {
    let node = root
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "location")
        .ok_or_else(|| Error::SumoModel("SUMO network missing <location>".to_owned()))?;
    let net_offset_raw = required_attr(node, "netOffset")?;
    let conv_boundary_raw = required_attr(node, "convBoundary")?;
    let net_offset = parse_pair(&net_offset_raw)?;
    let conv_boundary = parse_quad(&conv_boundary_raw)?;
    Ok(SumoLocation {
        net_offset,
        conv_boundary,
        net_offset_raw,
        conv_boundary_raw,
    })
}

fn parse_edge(
    edge: Node<'_, '_>,
    edges: &mut Vec<SumoEdge>,
    lanes: &mut Vec<SumoLane>,
) -> Result<()> {
    let edge_id = required_attr(edge, "id")?;
    let function_internal = edge.attribute("function") == Some("internal");
    edges.push(SumoEdge {
        id: edge_id.clone(),
        from_junction_id: edge.attribute("from").map(str::to_owned),
        to_junction_id: edge.attribute("to").map(str::to_owned),
        function_internal,
    });
    for lane in edge
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "lane")
    {
        let id = required_attr(lane, "id")?;
        let index = parse_u32(required_attr(lane, "index")?, "lane@index")?;
        let length = ExactDecimal::from_str(&required_attr(lane, "length")?)?;
        let speed = ExactDecimal::from_str(&required_attr(lane, "speed")?)?;
        let shape = parse_shape(&required_attr(lane, "shape")?)?;
        if shape.len() < 2 {
            return Err(Error::SumoModel(format!(
                "lane {id:?} shape must contain at least two points"
            )));
        }
        lanes.push(SumoLane {
            id,
            edge_id: edge_id.clone(),
            index,
            length,
            speed,
            shape,
            function_internal,
        });
    }
    Ok(())
}

fn parse_junction(node: Node<'_, '_>) -> Result<SumoJunction> {
    let id = required_attr(node, "id")?;
    let junction_type = required_attr(node, "type")?;
    let int_lane_ids = match node.attribute("intLanes") {
        Some(raw) if !raw.trim().is_empty() => raw
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    Ok(SumoJunction {
        id,
        junction_type,
        int_lane_ids,
    })
}

fn parse_connection(node: Node<'_, '_>) -> Result<SumoConnection> {
    let from_edge_id = required_attr(node, "from")?;
    let to_edge_id = required_attr(node, "to")?;
    let from_lane = parse_u32(required_attr(node, "fromLane")?, "connection@fromLane")?;
    let to_lane = parse_u32(required_attr(node, "toLane")?, "connection@toLane")?;
    let via_lane_ids = match node.attribute("via") {
        Some(via) if !via.trim().is_empty() => via
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let tl_id = node.attribute("tl").map(str::to_owned);
    let link_index = match node.attribute("linkIndex") {
        Some(raw) => Some(parse_u32(raw.to_owned(), "connection@linkIndex")?),
        None => None,
    };
    if tl_id.is_some() != link_index.is_some() {
        return Err(Error::SumoModel(format!(
            "connection {from_edge_id:?}->{to_edge_id:?} must set both @tl and @linkIndex or neither"
        )));
    }
    Ok(SumoConnection {
        from_edge_id,
        to_edge_id,
        from_lane,
        to_lane,
        via_lane_ids,
        tl_id,
        link_index,
    })
}

pub(crate) fn parse_tl_logic(node: Node<'_, '_>) -> Result<SumoTlLogic> {
    let id = required_attr(node, "id")?;
    let logic_type = required_attr(node, "type")?;
    let program_id = required_attr(node, "programID")?;
    let offset = ExactDecimal::from_str(&required_attr(node, "offset")?)?;
    let mut phases = Vec::new();
    for child in node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "phase")
    {
        phases.push(SumoTlPhase {
            duration: ExactDecimal::from_str(&required_attr(child, "duration")?)?,
            state: required_attr(child, "state")?,
        });
    }
    if phases.is_empty() {
        return Err(Error::SumoModel(format!(
            "tlLogic {id:?} has no <phase> children"
        )));
    }
    Ok(SumoTlLogic {
        id,
        logic_type,
        program_id,
        offset,
        phases,
    })
}

fn parse_shape(raw: &str) -> Result<Vec<(ExactDecimal, ExactDecimal)>> {
    let mut points = Vec::new();
    for token in raw.split_whitespace() {
        points.push(parse_pair(token)?);
    }
    Ok(points)
}

fn parse_pair(raw: &str) -> Result<(ExactDecimal, ExactDecimal)> {
    let (x, y) = raw.split_once(',').ok_or_else(|| {
        Error::SumoModel(format!("expected comma-separated pair, got {raw:?}"))
    })?;
    Ok((
        ExactDecimal::from_str(x.trim())?,
        ExactDecimal::from_str(y.trim())?,
    ))
}

fn parse_quad(
    raw: &str,
) -> Result<(ExactDecimal, ExactDecimal, ExactDecimal, ExactDecimal)> {
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() != 4 {
        return Err(Error::SumoModel(format!(
            "expected four comma-separated decimals, got {raw:?}"
        )));
    }
    Ok((
        ExactDecimal::from_str(parts[0].trim())?,
        ExactDecimal::from_str(parts[1].trim())?,
        ExactDecimal::from_str(parts[2].trim())?,
        ExactDecimal::from_str(parts[3].trim())?,
    ))
}

fn required_attr(node: Node<'_, '_>, name: &str) -> Result<String> {
    node.attribute(name).map(str::to_owned).ok_or_else(|| {
        Error::SumoModel(format!(
            "<{}> missing required attribute @{name}",
            node.tag_name().name()
        ))
    })
}

fn parse_u32(raw: String, field: &str) -> Result<u32> {
    raw.parse::<u32>()
        .map_err(|_| Error::SumoModel(format!("invalid u32 for {field}: {raw:?}")))
}
