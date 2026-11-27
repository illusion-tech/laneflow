use super::*;
use laneflow_static_contract::CanonicalFrameOrdinal;

type Result<T> = std::result::Result<T, BuildError>;

fn floats<const N: usize>(r: RegistryCheckedRowView<'_>, tag: u16) -> Result<[f32; N]> {
    let bytes = r.field_by_tag(tag).ok_or(INVALID)?.value_bytes();
    check(bytes.len() == 4 * N)?;
    Ok(std::array::from_fn(|i| {
        f32::from_le_bytes(
            bytes[i * 4..][..4]
                .try_into()
                .expect("checked float payload"),
        )
    }))
}
fn obstacle(shape: u8, b: &[u8]) -> Result<BoundaryPayload> {
    let word = |n: usize| b[n..n + 4].try_into().expect("checked parameter length");
    let i = |n| i32::from_le_bytes(word(n));
    let u = |n| u32::from_le_bytes(word(n));
    match (shape, b.len()) {
        (1, 25) => Ok(BoundaryPayload::Guardrail {
            anchor_role: b[0],
            lateral_mm: i(1),
            length_mm: u(5),
            radius_mm: u(9),
            heights_mm: [i(13), i(17)],
            error_mm: u(21),
        }),
        (2, 36) => Ok(BoundaryPayload::Curb {
            ends_mm: [[i(0), i(4), i(8), i(12)], [i(16), i(20), i(24), i(28)]],
            error_mm: u(32),
        }),
        _ => Err(INVALID),
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn build(
    view: ValueCheckedObjectView<'_>,
    traffic: &SharedTrafficNetwork,
    ranges: &[Option<RangeU32>],
    stations: &[u64],
    errors: &[u32],
    lanes: &[Option<Lane>],
    edges: &[Option<Edge>],
    bands: &[Band],
    extents: &mut [BoundaryExtent],
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(Box<[LaneFootprint]>, Box<[RoadSurfaceSpan]>)> {
    let root = view.registry_view();
    let section = root.section(8).ok_or(INVALID)?;
    let table = |i| section.table(i).ok_or(INVALID);
    let lane_rows = table(7)?;
    let patches = table(8)?;
    let mut footprints = allocate_vec(lane_rows.row_count(), STRUCTURE)?;
    for (i, r) in lane_rows.rows().enumerate() {
        let at = mapping_row(8, i as u32);
        poll_cancelled(options, i as u32)?;
        let n = index(r, 1)? as usize;
        let lane = AuthoringLaneOrdinal::try_from_usize(n).map_err(|_| at)?;
        validate(
            at,
            lanes.get(n).is_some_and(Option::is_some)
                && footprints
                    .last()
                    .is_none_or(|f: &LaneFootprint| f.lane < lane),
        )?;
        let d = |tag| offset_mm(r, tag);
        footprints.push(LaneFootprint {
            lane,
            ends_mm: [[d(2)?, d(3)?], [d(4)?, d(5)?]],
            error_mm: index(r, 6)?,
            supports_wheel: checked_u8(r, 7, STRUCTURE)? == 1,
        });
    }
    let mut rows: Vec<RoadSurfaceSpan> = allocate_vec(patches.row_count(), STRUCTURE)?;
    for (i, r) in patches.rows().enumerate() {
        let at = mapping_row(9, i as u32);
        poll_cancelled(options, i as u32)?;
        let corridor =
            RoadCorridorOrdinal::try_from_usize(index(r, 1)? as usize).map_err(|_| at)?;
        let domain = ranges.get(corridor.index()).copied().flatten().ok_or(at)?;
        let frame = index(r, 2)?;
        validate(
            at,
            frame < traffic.entity_counts().count(EntityKind::CanonicalFrame),
        )?;
        let frame = CanonicalFrameOrdinal::try_from_usize(frame as usize).map_err(|_| at)?;
        let lo = index(r, 3)?;
        let hi = index(r, 4)?;
        validate(
            at,
            domain.start() <= lo && lo < hi && hi < domain.start() + domain.len(),
        )?;
        let nodes = [lo as usize, hi as usize].map(|i| (stations[i], errors[i]));
        let jets = [floats(r, 5)?, floats(r, 6)?];
        for j in &jets {
            validate(at, j[3] != 0. || j[5] != 0.)?;
        }
        if let Some(prev) = rows.last() {
            validate(at, prev.corridor <= corridor)?;
            if prev.corridor == corridor {
                validate(
                    at,
                    prev.frame == frame
                        && prev.stations_mm[1] == nodes[0]
                        && prev.jets[1][..3] == jets[0][..3],
                )?;
                // 水平法向共用；真实纵坡缝可以有不同的单侧纵向导数。
                let [x, z] = [prev.jets[1][3], prev.jets[1][5]].map(f64::from);
                let [u, v] = [jets[0][3], jets[0][5]].map(f64::from);
                validate(at, x * v == z * u && x * u + z * v > 0.)?;
            } else {
                validate(at, lo == domain.start())?;
            }
        } else {
            validate(at, lo == domain.start())?;
        }
        rows.push(RoadSurfaceSpan {
            corridor,
            frame,
            stations_mm: nodes,
            jets,
            derivative_errors: floats(r, 7)?,
        });
    }

    let mut frames = if rows.is_empty() {
        Vec::new()
    } else {
        slots(ranges.len() as u32, None::<u32>)?
    };
    let mut checked_lanes = 0;
    let mut row_index = 0;
    for group in rows.chunk_by(|a, b| a.corridor == b.corridor) {
        let at = mapping_row(9, row_index);
        let c = group[0].corridor;
        frames[c.index()] = Some(group[0].frame.raw());
        let r = ranges[c.index()].ok_or(at)?;
        validate(
            at,
            group.last().ok_or(at)?.stations_mm[1].0
                == stations[(r.start() + r.len() - 1) as usize],
        )?;
        let mut seam = None;
        let mut next = |ends: [[i32; 2]; 2]| -> Result<()> {
            let left = ends.map(|e| e[0]);
            let right = ends.map(|e| e[1]);
            validate(at, seam.is_none_or(|last| last == left))?;
            seam = Some(right);
            Ok(())
        };
        for element in traffic.relations().corridor_elements(c).ok_or(at)? {
            match *element {
                CorridorElement::RoadSection(s) => {
                    for &l in traffic.relations().section_lanes(s).ok_or(at)? {
                        let i = footprints
                            .binary_search_by_key(&l, |r| r.lane)
                            .map_err(|_| at)?;
                        next(footprints[i].ends_mm)?;
                        checked_lanes += 1;
                    }
                }
                CorridorElement::FacilityBand(b) => {
                    let i = bands
                        .binary_search_by_key(&b, |r| r.ordinal)
                        .map_err(|_| at)?;
                    next(std::array::from_fn(|j| {
                        [bands[i].left_mm[j], bands[i].right_mm[j]]
                    }))?;
                }
            }
        }
        row_index += group.len() as u32;
    }
    check(checked_lanes == footprints.len())?;
    if !frames.is_empty() {
        let spatial = root.section(4).ok_or(INVALID)?;
        for (i, r) in spatial.table(1).ok_or(INVALID)?.rows().enumerate() {
            poll_cancelled(options, i as u32)?;
            if let Some(e) = edges.get(index(r, 1)? as usize).copied().flatten() {
                let c = lanes[e.lane.index()].ok_or(INVALID)?.corridor;
                if let Some(frame) = frames[c.index()] {
                    check(index(r, 2)? == frame)?;
                }
            }
        }
        for r in spatial.table(2).ok_or(INVALID)?.rows() {
            let n = index(r, 1)?;
            if let Ok(i) = bands.binary_search_by_key(&n, |b| b.ordinal.raw())
                && let Some(frame) = frames[bands[i].corridor.index()]
            {
                check(index(r, 2)? == frame)?;
            }
        }
    }
    for tag in [10u16, 11] {
        let mut previous = None;
        for (i, r) in table(u32::from(tag - 1))?.rows().enumerate() {
            poll_cancelled(options, i as u32)?;
            let at = mapping_row(tag, i as u32);
            let n = index(r, 1)?;
            let e = extents.get_mut(n as usize).ok_or(at)?;
            validate(
                at,
                previous.is_none_or(|p| p < n)
                    && e.kind == (tag - 9) as u8
                    && e.payload.is_none()
                    && frames.get(e.corridor.index()).is_some_and(Option::is_some),
            )?;
            previous = Some(n);
            e.payload = Some(if tag == 10 {
                BoundaryPayload::Rule {
                    directions: checked_u8(r, 2, STRUCTURE)?,
                    effect: checked_u8(r, 3, STRUCTURE)?,
                }
            } else {
                obstacle(
                    checked_u8(r, 2, STRUCTURE)?,
                    r.field_by_tag(3).ok_or(at)?.value_bytes(),
                )?
            });
        }
    }
    for (i, e) in extents.iter().enumerate() {
        validate(
            mapping_row(7, i as u32),
            e.payload.is_some() == frames.get(e.corridor.index()).is_some_and(Option::is_some),
        )?;
    }
    Ok((footprints.into_boxed_slice(), rows.into_boxed_slice()))
}
