use super::interval::Interval;
use super::surface::Projection;
use super::{Failure, TrackQuery, Work, partition};
use crate::{CorridorElement, LaneFootprint, SharedNetworkRevision};

fn edge(row: &LaneFootprint, side: usize, fraction: Interval) -> Interval {
    let [a, b] = row.ends_um.map(|ends| f64::from(ends[side]));
    let micrometers = Interval::point(a) + Interval::point(b - a) * fraction;
    (micrometers / Interval::point(1e6)).with_error(Interval::meters(u64::from(row.error_nm)).hi)
}

pub(super) enum Coverage {
    Inside,
    Outside(usize, Option<f64>),
    Uncertain(usize),
}

pub(super) fn classify(
    network: &SharedNetworkRevision,
    query: &TrackQuery<'_>,
    fractions: [Interval; 4],
    wheels: [Projection; 4],
    work: &mut Work,
) -> Result<Coverage, Failure> {
    let relations = network.traffic().relations();
    work.charge()?;
    let elements = relations
        .corridor_elements(query.corridor)
        .ok_or(Failure::Binding)?;
    let mut eligible = query.traversable_lanes.iter().peekable();
    let mut possible = [false; 4];
    let mut distance = [f64::INFINITY; 4];
    let mut inside = [false; 4];
    let mut group: Option<(&LaneFootprint, &LaneFootprint)> = None;
    let mut finish = |group: &mut Option<(&LaneFootprint, &LaneFootprint)>,
                      work: &mut Work|
     -> Result<(), Failure> {
        if let Some((first, last)) = group.take() {
            for (i, wheel) in wheels.iter().enumerate() {
                work.charge()?;
                let left = edge(first, 0, fractions[i]);
                let right = edge(last, 1, fractions[i]);
                let at = wheel.lateral;
                distance[i] = distance[i].min((right - at).lo.max((at - left).lo).max(0.));
                possible[i] |= at.hi >= right.lo && at.lo <= left.hi;
                inside[i] |= at.lo >= right.hi && at.hi <= left.lo;
            }
        }
        Ok(())
    };
    // 共边在共享构建时按有序成员验证。连续合格车道只比较并集的外缘，
    // 故轮迹跨越同一条带量化误差的内部接缝时也能得到连续覆盖证据。
    let footprints = &network.station_mapping().footprints;
    for element in elements {
        work.charge()?;
        let CorridorElement::RoadSection(section) = *element else {
            finish(&mut group, work)?;
            continue;
        };
        work.charge()?;
        for lane in relations.section_lanes(section).ok_or(Failure::Binding)? {
            work.charge()?;
            if eligible.next_if(|next| *next == lane).is_none() {
                finish(&mut group, work)?;
                continue;
            }
            let index = partition(footprints, |r| r.lane < *lane, work)?;
            work.charge()?;
            let row = footprints
                .get(index)
                .filter(|r| r.lane == *lane)
                .ok_or(Failure::Unavailable)?;
            if row.supports_wheel {
                group = Some((group.map_or(row, |(first, _)| first), row));
            } else {
                finish(&mut group, work)?;
            }
        }
    }
    if eligible.next().is_some() {
        return Err(Failure::Binding);
    }
    finish(&mut group, work)?;
    if let Some(i) = possible.iter().position(|&v| !v) {
        return Ok(Coverage::Outside(
            i,
            distance[i].is_finite().then_some(distance[i]),
        ));
    }
    Ok(inside
        .iter()
        .position(|&v| !v)
        .map_or(Coverage::Inside, Coverage::Uncertain))
}
