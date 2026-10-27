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

pub(super) fn probe(control: [Vec3; 6], start_clamp: Option<Vec3>) -> Value {
    let mut variants = BTreeMap::new();
    for variant in ["uniform", "start_dense", "finish_dense"] {
        let params = (0..=STEPS)
            .map(|i| {
                let u = i as f64 / STEPS as f64;
                match variant {
                    "uniform" => u,
                    "start_dense" => u * u,
                    "finish_dense" => 1.0 - (1.0 - u) * (1.0 - u),
                    _ => unreachable!(),
                }
            })
            .collect::<Vec<_>>();
        let mut state = GridSearch {
            start_clamp,
            audit: Some(EndAudit::default()),
            ..GridSearch::default()
        };
        let result = graph::paths_on(&mut state, control, None, &params);
        let paths = result.as_ref().ok().map(|paths| paths.iter().map(|p| {
            let mut sink = EmissionSink::new(control[0]);
            for &i in p.iter().skip(1) {
                sink.push(if i == STEPS { control[3] } else { cubic_at(control[0],control[1],control[2],control[3],params[i]) }).unwrap();
            }
            json!({"parameters_f64_bits":p.iter().map(|&i|params[i].to_bits()).collect::<Vec<_>>(),
                "arc_m":sink.total_arc_meters})
        }).collect::<Vec<_>>());
        let skeleton = state
            .audit
            .as_ref()
            .and_then(|a| a.graphs.last())
            .and_then(|g| g["closest_terminal_parameters_f64_bits"].as_array())
            .map(|v| {
                v.iter()
                    .map(|v| f64::from_bits(v.as_u64().unwrap()))
                    .collect::<Vec<_>>()
            });
        let refinement = if result.as_ref().is_ok_and(Vec::is_empty) {
            skeleton.map(|p| {
                let started=state.work();
                let found=refine::search(&mut state,control,None,&p);
                if let Ok(paths) = &found {
                    for path in paths {
                        let mut sink = EmissionSink::new(control[0]);
                        for &t in path.iter().skip(1) {
                            sink.push(if t == 1.0 { control[3] } else { cubic_at(control[0], control[1], control[2], control[3], t) }).unwrap();
                        }
                    }
                }
                json!({"skeleton_parameters_f64_bits":p.iter().map(|t|t.to_bits()).collect::<Vec<_>>(),
                    "parameters_f64_bits":found.as_ref().ok().map(|paths|paths.iter().map(|p|p.iter().map(|t|t.to_bits()).collect::<Vec<_>>()).collect::<Vec<_>>()),
                    "error":found.err().map(|e|e.to_string()),"additional_work":state.work()-started})
            })
        } else {
            None
        };
        variants.insert(
            variant,
            json!({"paths":paths,"refinement":refinement,"error":result.err().map(|e|e.to_string()),
            "counts":state.counts,"work":state.work(),"stop":state.stop,"audit":state.audit}),
        );
    }
    let c = [control[0], control[1], control[2], control[3]];
    let a = quantize_vec3(c[0]);
    let mut minimum: Option<(f64, Value)> = None;
    let mut eligible = 0;
    for k in 1..=65_536 {
        let t = f64::from(k) / 65_536.0;
        let b = quantize_vec3(if k == 65_536 {
            c[3]
        } else {
            cubic_at(c[0], c[1], c[2], c[3], t)
        });
        let length = quantized_chord_meters(a, b);
        if length <= QUANTIZED_MIN_SEGMENT_METERS {
            continue;
        }
        let deviation = graph::deviation(c, 0.0, t, a.map(f64::from), b.map(f64::from));
        if deviation > DEVIATION_METERS {
            continue;
        }
        eligible += 1;
        let angle = angle_rad(
            unit(sub(b.map(f64::from), a.map(f64::from))).unwrap(),
            start_clamp.unwrap_or(control[4]),
        )
        .to_degrees();
        if minimum.as_ref().is_none_or(|(best, _)| angle < *best) {
            minimum = Some((
                angle,
                json!({"parameter_f64_bits":t.to_bits(),"angle_degrees":angle,
                "quantized_end_f32_bits":b.map(f32::to_bits),"chord_m":length,"deviation_estimate_m":deviation}),
            ));
        }
    }
    json!({"control_f64_bits":control.map(|p|p.map(f64::to_bits)),"start_clamp":start_clamp.map(|d|d.map(f64::to_bits)),
        "variants":variants,"dense_first_chord_probe":{"steps":65_536,"eligible_chords":eligible,"minimum_angle":minimum.map(|(_,v)|v),"continuous_certificate":false},
        "scope":"finite local cubic reachability; excludes downstream spans and controller acceptance"})
}
