use super::*;
use runtime::{FlatBufferBuilder, ForwardsUOffset, UnionWIPOffset, Vector, WIPOffset};

type Extents<'a> = WIPOffset<Vector<'a, ForwardsUOffset<wire::BoundaryExtent<'a>>>>;

fn member<'a>(
    fbb: &mut FlatBufferBuilder<'a>,
    value: &BoundaryMemberInput,
) -> WIPOffset<wire::BoundaryMember<'a>> {
    use wire::BoundaryMemberKind as Kind;
    let (kind, reference) = match value {
        BoundaryMemberInput::RoadSection(r) => (Kind::RoadSection, create_reference(fbb, r)),
        BoundaryMemberInput::FacilityBand(r) => (Kind::FacilityBand, create_reference(fbb, r)),
        BoundaryMemberInput::AuthoringLane(r) => (Kind::AuthoringLane, create_reference(fbb, r)),
    };
    wire::BoundaryMember::create(
        fbb,
        &wire::BoundaryMemberArgs {
            kind,
            entity_reference: Some(reference),
        },
    )
}

fn station(value: BoundaryStation) -> wire::BoundaryStation {
    match value {
        BoundaryStation::Fixed(s) => {
            wire::BoundaryStation::new(wire::BoundaryStationKind::Fixed, s)
        }
        BoundaryStation::CorridorEndpoint => {
            wire::BoundaryStation::new(wire::BoundaryStationKind::CorridorEndpoint, 0.0)
        }
    }
}

fn curb_end(e: CurbEndInput) -> wire::CurbEnd {
    wire::CurbEnd::new(
        e.left_offset_meters,
        e.right_offset_meters,
        e.bottom_height_meters,
        e.top_height_meters,
    )
}

fn payload(
    fbb: &mut FlatBufferBuilder<'_>,
    value: Option<BoundaryPayloadInput>,
) -> (wire::BoundaryPayload, Option<WIPOffset<UnionWIPOffset>>) {
    let (kind, offset) = match value {
        None => return (wire::BoundaryPayload::NONE, None),
        Some(BoundaryPayloadInput::CrossingRule { directions, effect }) => (
            wire::BoundaryPayload::CrossingRule,
            wire::CrossingRule::create(
                fbb,
                &wire::CrossingRuleArgs {
                    directions: Some(directions),
                    effect: Some(effect),
                },
            )
            .as_union_value(),
        ),
        Some(BoundaryPayloadInput::Guardrail {
            anchor_role,
            lateral_offset_meters,
            horizontal_length_meters,
            radius_meters,
            bottom_height_meters,
            top_height_meters,
            parameter_error_meters,
        }) => (
            wire::BoundaryPayload::Guardrail,
            wire::Guardrail::create(
                fbb,
                &wire::GuardrailArgs {
                    anchor_role: Some(anchor_role),
                    lateral_offset_meters: Some(lateral_offset_meters),
                    horizontal_length_meters: Some(horizontal_length_meters),
                    radius_meters: Some(radius_meters),
                    bottom_height_meters: Some(bottom_height_meters),
                    top_height_meters: Some(top_height_meters),
                    parameter_error_meters: Some(parameter_error_meters),
                },
            )
            .as_union_value(),
        ),
        Some(BoundaryPayloadInput::Curb {
            low,
            high,
            parameter_error_meters,
        }) => (
            wire::BoundaryPayload::Curb,
            wire::Curb::create(
                fbb,
                &wire::CurbArgs {
                    low: Some(&curb_end(low)),
                    high: Some(&curb_end(high)),
                    parameter_error_meters: Some(parameter_error_meters),
                },
            )
            .as_union_value(),
        ),
    };
    (kind, Some(offset))
}

pub(super) fn encode_extents<'a>(
    fbb: &mut FlatBufferBuilder<'a>,
    values: &[BoundaryExtentInput],
) -> Extents<'a> {
    let entries = values
        .iter()
        .map(|e| {
            let left = member(fbb, &e.left);
            let right = member(fbb, &e.right);
            let key = fbb.create_string(&e.local_key);
            let canvas = create_canvas(fbb, e.canvas_selection.as_deref());
            let (payload_type, payload) = payload(fbb, e.payload);
            wire::BoundaryExtent::create(
                fbb,
                &wire::BoundaryExtentArgs {
                    left: Some(left),
                    right: Some(right),
                    extent_kind: match e.kind {
                        BoundaryExtentKind::Rule => wire::BoundaryExtentKind::Rule,
                        BoundaryExtentKind::Obstacle => wire::BoundaryExtentKind::Obstacle,
                    },
                    local_key: Some(key),
                    low: Some(&station(e.low)),
                    high: Some(&station(e.high)),
                    payload_type,
                    payload,
                    canvas_selection: canvas,
                },
            )
        })
        .collect::<Vec<_>>();
    fbb.create_vector(&entries)
}
