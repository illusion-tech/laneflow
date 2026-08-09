//! ScenarioManifest 0.1 与 SpatialPackage 0.1 的单遍解码。

use serde_json::value::RawValue;

use super::walk::{self, Ctx, LocationPolicy, ShapeCandidate};
use super::{ByteRange, GateReport, ParseFailure, RootGate, missing_root_field};
use crate::scenario_wire::{
    WireArtifactDescriptor, WireCenterline, WireScenarioManifest, WireSpatialEdge,
    WireSpatialPackage,
};
use crate::{CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION, CURRENT_SPATIAL_FORMAT_VERSION};

const MANIFEST_FIELDS: &[&str] = &["formatVersion", "traffic", "spatial"];
const DESCRIPTOR_FIELDS: &[&str] = &["artifactRef", "mediaType", "digest", "size"];
const SPATIAL_FIELDS: &[&str] = &["formatVersion", "frameId", "edges"];
const EDGE_FIELDS: &[&str] = &["trafficEdgeId", "centerline"];
const CENTERLINE_FIELDS: &[&str] = &["points"];

/// 解析 ScenarioManifest wire（单遍：闸口 + 完整 shape）。
pub(crate) fn parse_manifest(input: &[u8]) -> Result<WireScenarioManifest, ParseFailure> {
    let mut fields = ManifestFields::default();
    let GateReport { gate, root_range } = super::drive_root(
        input,
        CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION,
        "struct WireScenarioManifest",
        MANIFEST_FIELDS,
        |ctx, key, value, range, mark, gate| {
            fields.handle(ctx, key, value, range, mark, gate);
        },
    )?;
    if let Some(candidate) = gate.deferred {
        return Err(ParseFailure::Shape(candidate));
    }
    let format_version = gate.format_version.expect("闸口保证版本字段存在");
    ManifestFields::finish(fields, format_version, root_range)
}

/// 解析 SpatialPackage wire（单遍：闸口 + 完整 shape）。
pub(crate) fn parse_spatial(input: &[u8]) -> Result<WireSpatialPackage, ParseFailure> {
    let mut fields = SpatialFields::default();
    let GateReport { gate, root_range } = super::drive_root(
        input,
        CURRENT_SPATIAL_FORMAT_VERSION,
        "struct WireSpatialPackage",
        SPATIAL_FIELDS,
        |ctx, key, value, range, mark, gate| {
            fields.handle(ctx, key, value, range, mark, gate);
        },
    )?;
    if let Some(candidate) = gate.deferred {
        return Err(ParseFailure::Shape(candidate));
    }
    let format_version = gate.format_version.expect("闸口保证版本字段存在");
    SpatialFields::finish(fields, format_version, root_range)
}

#[derive(Default)]
struct ManifestFields {
    traffic: Option<WireArtifactDescriptor>,
    spatial: Option<WireArtifactDescriptor>,
}

impl ManifestFields {
    fn handle<'de>(
        &mut self,
        ctx: &mut Ctx<'de, impl LocationPolicy>,
        key: &str,
        value: &'de RawValue,
        range: ByteRange,
        mark: usize,
        gate: &mut RootGate,
    ) {
        let result = match key {
            "traffic" => walk::set_once(
                ctx,
                &mut self.traffic,
                "traffic",
                value,
                range,
                mark,
                |ctx, token, range| decode_descriptor(ctx, token, range),
            ),
            "spatial" => walk::set_once(
                ctx,
                &mut self.spatial,
                "spatial",
                value,
                range,
                mark,
                |ctx, token, range| decode_descriptor(ctx, token, range),
            ),
            _ => Err(ctx.candidate(walk::unknown_field_message(key, MANIFEST_FIELDS), range)),
        };
        if let Err(candidate) = result {
            gate.defer(candidate);
        }
    }

    fn finish(
        self,
        format_version: String,
        root_range: ByteRange,
    ) -> Result<WireScenarioManifest, ParseFailure> {
        Ok(WireScenarioManifest {
            format_version,
            traffic: self
                .traffic
                .ok_or_else(|| missing_root_field("traffic", root_range))?,
            spatial: self
                .spatial
                .ok_or_else(|| missing_root_field("spatial", root_range))?,
        })
    }
}

fn decode_descriptor<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireArtifactDescriptor, ShapeCandidate> {
    let mut artifact_ref = None;
    let mut media_type = None;
    let mut digest = None;
    let mut size = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireArtifactDescriptor",
        DESCRIPTOR_FIELDS,
        |ctx, key, value, value_range, mark| match key {
            "artifactRef" => walk::set_once(
                ctx,
                &mut artifact_ref,
                "artifactRef",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "mediaType" => walk::set_once(
                ctx,
                &mut media_type,
                "mediaType",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "digest" => walk::set_once(
                ctx,
                &mut digest,
                "digest",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "size" => walk::set_once(
                ctx,
                &mut size,
                "size",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, DESCRIPTOR_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireArtifactDescriptor {
        artifact_ref: artifact_ref
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("artifactRef"), range))?,
        media_type: media_type
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("mediaType"), range))?,
        digest: digest
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("digest"), range))?,
        size: size.ok_or_else(|| ctx.candidate(walk::missing_field_message("size"), range))?,
    })
}

#[derive(Default)]
struct SpatialFields {
    frame_id: Option<String>,
    edges: Option<Vec<WireSpatialEdge>>,
}

impl SpatialFields {
    fn handle<'de>(
        &mut self,
        ctx: &mut Ctx<'de, impl LocationPolicy>,
        key: &str,
        value: &'de RawValue,
        range: ByteRange,
        mark: usize,
        gate: &mut RootGate,
    ) {
        let result = match key {
            "frameId" => walk::set_once(
                ctx,
                &mut self.frame_id,
                "frameId",
                value,
                range,
                mark,
                walk::decode_scalar,
            ),
            "edges" => walk::set_once(
                ctx,
                &mut self.edges,
                "edges",
                value,
                range,
                mark,
                decode_edges,
            ),
            _ => Err(ctx.candidate(walk::unknown_field_message(key, SPATIAL_FIELDS), range)),
        };
        if let Err(candidate) = result {
            gate.defer(candidate);
        }
    }

    fn finish(
        self,
        format_version: String,
        root_range: ByteRange,
    ) -> Result<WireSpatialPackage, ParseFailure> {
        Ok(WireSpatialPackage {
            format_version,
            frame_id: self
                .frame_id
                .ok_or_else(|| missing_root_field("frameId", root_range))?,
            edges: self
                .edges
                .ok_or_else(|| missing_root_field("edges", root_range))?,
        })
    }
}

fn decode_edges<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<Vec<WireSpatialEdge>, ShapeCandidate> {
    let mut edges = Vec::new();
    walk::decode_array(
        ctx,
        token,
        range,
        "a sequence",
        |ctx, _index, element, element_range| {
            edges.push(decode_edge(ctx, element, element_range)?);
            Ok(())
        },
    )?;
    Ok(edges)
}

fn decode_edge<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireSpatialEdge, ShapeCandidate> {
    let mut traffic_edge_id = None;
    let mut centerline = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireSpatialEdge",
        EDGE_FIELDS,
        |ctx, key, value, value_range, mark| match key {
            "trafficEdgeId" => walk::set_once(
                ctx,
                &mut traffic_edge_id,
                "trafficEdgeId",
                value,
                value_range,
                mark,
                walk::decode_scalar,
            ),
            "centerline" => walk::set_once(
                ctx,
                &mut centerline,
                "centerline",
                value,
                value_range,
                mark,
                decode_centerline,
            ),
            _ => Err(ctx.candidate(walk::unknown_field_message(key, EDGE_FIELDS), value_range)),
        },
    )?;
    Ok(WireSpatialEdge {
        traffic_edge_id: traffic_edge_id
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("trafficEdgeId"), range))?,
        centerline: centerline
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("centerline"), range))?,
    })
}

fn decode_centerline<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<WireCenterline, ShapeCandidate> {
    let mut points = None;
    walk::decode_record(
        ctx,
        token,
        range,
        "struct WireCenterline",
        CENTERLINE_FIELDS,
        |ctx, key, value, value_range, mark| match key {
            "points" => walk::set_once(
                ctx,
                &mut points,
                "points",
                value,
                value_range,
                mark,
                decode_points,
            ),
            _ => Err(ctx.candidate(
                walk::unknown_field_message(key, CENTERLINE_FIELDS),
                value_range,
            )),
        },
    )?;
    Ok(WireCenterline {
        points: points
            .ok_or_else(|| ctx.candidate(walk::missing_field_message("points"), range))?,
    })
}

fn decode_points<'de, L: LocationPolicy>(
    ctx: &mut Ctx<'de, L>,
    token: &'de RawValue,
    range: ByteRange,
) -> Result<Vec<[f64; 3]>, ShapeCandidate> {
    let mut points = Vec::new();
    walk::decode_array(
        ctx,
        token,
        range,
        "a sequence",
        |ctx, _index, element, element_range| {
            points.push(walk::decode_point(ctx, element, element_range)?);
            Ok(())
        },
    )?;
    Ok(points)
}
