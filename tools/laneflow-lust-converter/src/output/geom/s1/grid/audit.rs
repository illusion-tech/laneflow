//! #1111：完整边端弦裁决的观察记录；不参与候选顺序和预算。

use super::*;

#[derive(Default, Serialize)]
pub(super) struct EndAudit {
    complete_edges: u64,
    start_only_rejected: u64,
    finish_only_rejected: u64,
    both_rejected: u64,
    accepted: u64,
    angle_ranges_degrees: [Option<[f64; 2]>; 2],
    first_eight: Vec<Value>,
    graphs: Vec<Value>,
}

impl EndAudit {
    pub(super) fn graph(&mut self, graph: Value) {
        if self.graphs.len() < 64 {
            self.graphs.push(graph);
        }
    }

    pub(super) fn record(&mut self, clamps: [Option<Vec3>; 2], dirs: [Vec3; 2], spans: &[Value]) {
        self.complete_edges += 1;
        let angles = [0, 1].map(|i| clamps[i].map(|c| angle_rad(dirs[i], c).to_degrees()));
        let rejected = angles.map(|a| a.is_some_and(|a| a > END_DEGREES));
        match rejected {
            [true, false] => self.start_only_rejected += 1,
            [false, true] => self.finish_only_rejected += 1,
            [true, true] => self.both_rejected += 1,
            [false, false] => self.accepted += 1,
        }
        for (range, angle) in self.angle_ranges_degrees.iter_mut().zip(angles) {
            if let Some(angle) = angle {
                let r = range.get_or_insert([angle; 2]);
                r[0] = r[0].min(angle);
                r[1] = r[1].max(angle);
            }
        }
        if self.first_eight.len() < 8 {
            self.first_eight
                .push(json!({"angles_degrees":angles,"rejected":rejected,
                "directions_f64_bits":dirs.map(|d|d.map(f64::to_bits)),
                "first_span":spans.first(),"last_span":spans.last()}));
        }
    }
}
