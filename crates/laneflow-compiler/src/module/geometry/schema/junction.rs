//! Junction、internal edge 与 connection 的紧凑 wire records。

use super::road::{
    ParsedCurve, RawNumber, parse_array, parse_curve, parse_raw_number, parse_unique_tokens,
};
use super::{
    ByteSpan, ClosedFields, JsonCursor, SchemaError, SpannedString, parse_object_members,
    parse_token,
};

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedJunctionRecord {
    pub(in crate::module::geometry) junction_key: SpannedString,
    pub(in crate::module::geometry) approach_edges: Box<[SpannedString]>,
    pub(in crate::module::geometry) internal_edges: Box<[ParsedInternalEdge]>,
    pub(in crate::module::geometry) connections: Box<[ParsedConnection]>,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedInternalEdge {
    pub(in crate::module::geometry) lane_edge_key: SpannedString,
    pub(in crate::module::geometry) speed_limit_meters_per_second: RawNumber,
    pub(in crate::module::geometry) geometry: ParsedCurve,
    pub(in crate::module::geometry) span: ByteSpan,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct ParsedConnection {
    pub(in crate::module::geometry) movement_key: SpannedString,
    pub(in crate::module::geometry) directed_entry_approach_key: SpannedString,
    pub(in crate::module::geometry) directed_exit_approach_key: SpannedString,
    pub(in crate::module::geometry) maneuver_path_key: SpannedString,
    pub(in crate::module::geometry) entry_edge: SpannedString,
    pub(in crate::module::geometry) internal_edge_sequence: Box<[SpannedString]>,
    pub(in crate::module::geometry) exit_edge: SpannedString,
    pub(in crate::module::geometry) span: ByteSpan,
}

pub(in crate::module::geometry) fn parse_junction_records(
    cursor: &mut JsonCursor<'_>,
) -> Result<Box<[ParsedJunctionRecord]>, SchemaError> {
    parse_array(cursor, "junctions", false, parse_junction_record)
}

fn parse_junction_record(cursor: &mut JsonCursor<'_>) -> Result<ParsedJunctionRecord, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "junctionKey",
        "approachEdges",
        "internalEdges",
        "connections",
    ]);
    let mut junction_key = None;
    let mut approach_edges = None;
    let mut internal_edges = None;
    let mut connections = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => junction_key = Some(parse_token(cursor, "junctionKey")?),
            1 => approach_edges = Some(parse_unique_tokens(cursor, "approachEdges")?),
            2 => {
                internal_edges = Some(parse_array(
                    cursor,
                    "internalEdges",
                    false,
                    parse_internal_edge,
                )?)
            }
            3 => connections = Some(parse_array(cursor, "connections", true, parse_connection)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    let approach_edges = approach_edges.expect("required field checked");
    if approach_edges.len() < 2 {
        return Err(SchemaError {
            kind: super::SchemaErrorKind::WrongArrayLength {
                field: "approachEdges",
                expected: 2,
                actual: approach_edges.len(),
            },
            span,
        });
    }
    Ok(ParsedJunctionRecord {
        junction_key: junction_key.expect("required field checked"),
        approach_edges,
        internal_edges: internal_edges.expect("required field checked"),
        connections: connections.expect("required field checked"),
        span,
    })
}

fn parse_internal_edge(cursor: &mut JsonCursor<'_>) -> Result<ParsedInternalEdge, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new(["laneEdgeKey", "speedLimitMetersPerSecond", "geometry"]);
    let mut lane_edge_key = None;
    let mut speed_limit = None;
    let mut geometry = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => lane_edge_key = Some(parse_token(cursor, "laneEdgeKey")?),
            1 => speed_limit = Some(parse_raw_number(cursor)?),
            2 => geometry = Some(parse_curve(cursor)?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedInternalEdge {
        lane_edge_key: lane_edge_key.expect("required field checked"),
        speed_limit_meters_per_second: speed_limit.expect("required field checked"),
        geometry: geometry.expect("required field checked"),
        span,
    })
}

fn parse_connection(cursor: &mut JsonCursor<'_>) -> Result<ParsedConnection, SchemaError> {
    let start = cursor.begin_object()?.start;
    let mut fields = ClosedFields::new([
        "movementKey",
        "directedEntryApproachKey",
        "directedExitApproachKey",
        "maneuverPathKey",
        "entryEdge",
        "internalEdgeSequence",
        "exitEdge",
    ]);
    let mut movement_key = None;
    let mut entry_approach = None;
    let mut exit_approach = None;
    let mut path_key = None;
    let mut entry_edge = None;
    let mut internal_edges = None;
    let mut exit_edge = None;
    parse_object_members(cursor, &mut fields, |index, cursor| {
        match index {
            0 => movement_key = Some(parse_token(cursor, "movementKey")?),
            1 => entry_approach = Some(parse_token(cursor, "directedEntryApproachKey")?),
            2 => exit_approach = Some(parse_token(cursor, "directedExitApproachKey")?),
            3 => path_key = Some(parse_token(cursor, "maneuverPathKey")?),
            4 => entry_edge = Some(parse_token(cursor, "entryEdge")?),
            5 => internal_edges = Some(parse_unique_tokens(cursor, "internalEdgeSequence")?),
            6 => exit_edge = Some(parse_token(cursor, "exitEdge")?),
            _ => unreachable!(),
        }
        Ok(())
    })?;
    let end = cursor.end_object()?.end;
    let span = ByteSpan { start, end };
    fields.require_all(span)?;
    Ok(ParsedConnection {
        movement_key: movement_key.expect("required field checked"),
        directed_entry_approach_key: entry_approach.expect("required field checked"),
        directed_exit_approach_key: exit_approach.expect("required field checked"),
        maneuver_path_key: path_key.expect("required field checked"),
        entry_edge: entry_edge.expect("required field checked"),
        internal_edge_sequence: internal_edges.expect("required field checked"),
        exit_edge: exit_edge.expect("required field checked"),
        span,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_junction_records;
    use crate::module::geometry::json::JsonCursor;
    use crate::module::geometry::schema::SchemaErrorKind;

    #[test]
    fn parses_shared_internal_edge_connections_and_geometry() {
        let source = br#"[{"junctionKey":"junction.main","approachEdges":["edge.in","edge.out"],"internalEdges":[{"laneEdgeKey":"edge.internal","speedLimitMetersPerSecond":8,"geometry":{"start":[0,0,0],"segments":[{"kind":"line","end":[5,0,5]}]}}],"connections":[{"movementKey":"movement.a","directedEntryApproachKey":"approach.in","directedExitApproachKey":"approach.out","maneuverPathKey":"path.a","entryEdge":"edge.in","internalEdgeSequence":["edge.internal"],"exitEdge":"edge.out"},{"movementKey":"movement.b","directedEntryApproachKey":"approach.in","directedExitApproachKey":"approach.out","maneuverPathKey":"path.b","entryEdge":"edge.in","internalEdgeSequence":["edge.internal"],"exitEdge":"edge.out"}]}]"#;
        let mut cursor = JsonCursor::new(source).unwrap();
        let junctions = parse_junction_records(&mut cursor).unwrap();
        assert_eq!(junctions.len(), 1);
        assert_eq!(junctions[0].connections.len(), 2);
        assert_eq!(junctions[0].internal_edges.len(), 1);
    }

    #[test]
    fn rejects_duplicate_or_too_few_approaches_and_empty_connections() {
        let mut duplicate = JsonCursor::new(
            br#"[{"junctionKey":"j","approachEdges":["e","e"],"internalEdges":[],"connections":[{}]}]"#,
        )
        .unwrap();
        assert!(matches!(
            parse_junction_records(&mut duplicate).unwrap_err().kind,
            SchemaErrorKind::DuplicateArrayItem {
                field: "approachEdges",
                ..
            }
        ));

        let mut too_few = JsonCursor::new(
            br#"[{"junctionKey":"j","approachEdges":["e"],"internalEdges":[],"connections":[]}]"#,
        )
        .unwrap();
        assert!(matches!(
            parse_junction_records(&mut too_few).unwrap_err().kind,
            SchemaErrorKind::EmptyArray("connections")
        ));
    }
}
