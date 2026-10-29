//! #1084：测试显式开启的真实递归采样观察；生产构建不包含本模块。

use std::cell::RefCell;

use serde_json::{Value, json};

use super::{EmissionSink, Vec3};

#[derive(Default)]
struct Observation {
    outer_span: [usize; 2],
    stack: Vec<usize>,
    calls: Vec<Value>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Observation>> = const { RefCell::new(None) };
}

struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        ACTIVE.with(|slot| *slot.borrow_mut() = None);
    }
}

pub(crate) fn capture<T>(run: impl FnOnce() -> T) -> (T, Vec<Value>) {
    ACTIVE.with(|slot| {
        assert!(slot.borrow().is_none(), "nested sampling observation");
        *slot.borrow_mut() = Some(Observation::default());
    });
    let _reset = Reset;
    let result = run();
    let observation = ACTIVE.with(|slot| slot.borrow_mut().take().expect("observation"));
    assert!(observation.stack.is_empty());
    (result, observation.calls)
}

pub(super) fn outer_span(index: usize, count: usize) {
    ACTIVE.with(|slot| {
        if let Some(observation) = slot.borrow_mut().as_mut() {
            assert!(observation.stack.is_empty());
            observation.outer_span = [index, count];
        }
    });
}

pub(super) fn enter(control: [Vec3; 6], sink: &EmissionSink) -> Option<usize> {
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let observation = slot.as_mut()?;
        let id = observation.calls.len();
        let [a, c1, c2, b, dir_a, dir_b] = control;
        observation.calls.push(json!({
            "call": id, "parent": observation.stack.last(), "depth": observation.stack.len(),
            "span_index_zero_based": observation.outer_span[0], "span_count": observation.outer_span[1],
            "a": a, "c1": c1, "c2": c2, "b": b, "dir_a": dir_a, "dir_b": dir_b,
            "control_f64_bits": control.map(|point| point.map(f64::to_bits)),
            "sink_segments_before": sink.segments.len(),
            "sink_previous_point_f32_bits": sink.prev_point.map(f32::to_bits),
            "sink_previous_direction_f64_bits": sink.prev_dir.map(|point| point.map(f64::to_bits)),
            "sink_arc_f64_bits": sink.total_arc_meters.to_bits(),
        }));
        observation.stack.push(id);
        Some(id)
    })
}

pub(super) fn finish(id: Option<usize>, result: &crate::Result<()>) {
    if let Some(id) = id {
        ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let observation = slot.as_mut().expect("active observation");
            assert_eq!(observation.stack.pop(), Some(id));
            observation.calls[id]["result"] = match result {
                Ok(()) => json!({"status": "accepted"}),
                Err(error) => json!({"status": "rejected", "error": error.to_string()}),
            };
        });
    }
}
