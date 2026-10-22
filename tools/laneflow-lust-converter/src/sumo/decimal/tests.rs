use std::path::Path;

use super::*;
use crate::{
    source::{LUST_COMMIT, read_verified, recheck_source_revision, verify_source_dir},
    sumo::{
        parse_due_routes_xml_filtered, parse_sumo_network_xml, parse_tll_static_xml,
        parse_vtypes_xml,
    },
};

// 独立保留堆字符串与头部插零路径，作 #996 的逐字节参考；不进入生产实现。
fn reference_text(value: ExactDecimal) -> String {
    if value.scale == 0 {
        return value.digits.to_string();
    }
    let negative = value.digits < 0;
    let mut digits = value.digits.unsigned_abs().to_string();
    while digits.len() <= value.scale as usize {
        digits.insert(0, '0');
    }
    let split = digits.len() - value.scale as usize;
    let mut text = String::new();
    if negative {
        text.push('-');
    }
    text.push_str(&digits[..split]);
    text.push('.');
    text.push_str(&digits[split..]);
    text
}

fn assert_same_rendering(value: ExactDecimal) {
    let expected = reference_text(value);
    assert_eq!(value.to_plain_string(), expected, "decimal text: {value:?}");
    assert_eq!(
        value.to_f64().expect("binary64").to_bits(),
        expected
            .parse::<f64>()
            .expect("reference binary64")
            .to_bits(),
        "binary64 bits: {value:?}"
    );
}

#[test]
fn simplified_lust_origin_subtraction_matches_contract() {
    let sx = ExactDecimal::from_str("6806.88").expect("sx");
    let sy = ExactDecimal::from_str("5727.52").expect("sy");
    let ox = ExactDecimal::from_str("6806.88").expect("ox");
    let oz = ExactDecimal::from_str("5727.52").expect("oz");
    assert_eq!(sx.checked_sub(ox).expect("x").to_f64().expect("xf"), 0.0);
    assert_eq!(sy.checked_sub(oz).expect("z").to_f64().expect("zf"), 0.0);
}

#[test]
fn negative_zero_normalizes_to_positive_zero() {
    let decimal = ExactDecimal::from_str("-0.00").expect("parse");
    assert_eq!(decimal.to_plain_string(), "0");
    assert_eq!(decimal.to_f64().expect("f64").to_bits(), 0.0_f64.to_bits());
}

#[test]
fn rendering_covers_sign_padding_and_i128_boundaries() {
    for (digits, scale, expected) in [
        (0, 0, "0"),
        (1, 0, "1"),
        (-1, 0, "-1"),
        (123, 1, "12.3"),
        (-123, 2, "-1.23"),
        (123, 3, "0.123"),
        (-123, 5, "-0.00123"),
        (1, 18, "0.000000000000000001"),
        (i128::MIN, 0, "-170141183460469231731687303715884105728"),
        (i128::MAX, 0, "170141183460469231731687303715884105727"),
        (i128::MIN, 38, "-1.70141183460469231731687303715884105728"),
        (i128::MAX, 39, "0.170141183460469231731687303715884105727"),
    ] {
        let value = ExactDecimal { digits, scale };
        assert_eq!(value.to_plain_string(), expected);
        assert_same_rendering(value);
    }
}

#[test]
fn rendering_matches_reference_across_integer_widths_and_scales() {
    let mut state = 0x243f_6a88_85a3_08d3_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        state
    };
    for digits in [i128::MIN, i128::MAX, -10, -1, 0, 1, 9, 10, 100]
        .into_iter()
        .chain((0..1_024).map(|_| ((u128::from(next()) << 64) | u128::from(next())) as i128))
    {
        // midpoint 可产生超过 parser 18 位限制、甚至超过 i128 十进制位宽的 scale。
        for scale in 0..=76 {
            assert_same_rendering(ExactDecimal { digits, scale });
        }
    }
}

#[test]
fn repeated_midpoints_keep_precision_beyond_the_parser_scale() {
    let mut value = ExactDecimal::from_str("0.000000000000000001").expect("parse");
    for _ in 0..21 {
        value = value.midpoint(ExactDecimal::zero()).expect("midpoint");
        assert_same_rendering(value);
    }
    assert!(value.scale > 38);
}

fn pinned_lust_values(root: &Path) -> Vec<ExactDecimal> {
    let verified = verify_source_dir(root).expect("verify pinned source set");
    let xml = read_verified(&verified, "scenario/lust.net.xml").expect("read net");
    let network = parse_sumo_network_xml(&xml).expect("parse net");
    assert_eq!(network.external_lane_count(), 8_622);
    let location = &network.location;
    let (min_x, min_y, max_x, max_y) = location.conv_boundary;
    let origin = location.canonical_origin().expect("canonical origin");
    let mut values = vec![
        location.net_offset.0,
        location.net_offset.1,
        min_x,
        min_y,
        max_x,
        max_y,
        origin.0,
        origin.1,
    ];
    for lane in &network.lanes {
        values.extend([lane.length, lane.speed]);
        let first = lane.shape[0];
        for &(sx, sy) in &lane.shape {
            let px = sx.checked_sub(location.net_offset.0).expect("project x");
            let py = sy.checked_sub(location.net_offset.1).expect("project y");
            values.extend([
                sx,
                sy,
                px,
                py,
                px.checked_sub(origin.0).expect("center x"),
                py.checked_sub(origin.1).expect("center z"),
                sx.checked_sub(first.0).expect("relative x"),
                sy.checked_sub(first.1).expect("relative y"),
            ]);
        }
        for pair in lane.shape.windows(2) {
            values.extend([
                pair[1].0.checked_sub(pair[0].0).expect("delta x"),
                pair[1].1.checked_sub(pair[0].1).expect("delta y"),
            ]);
        }
    }
    let tll = read_verified(&verified, "scenario/tll.static.xml").expect("read signals");
    let static_logics = parse_tll_static_xml(&tll).expect("parse signals");
    for logic in network.tl_logics.iter().chain(&static_logics) {
        values.push(logic.offset);
        values.extend(logic.phases.iter().map(|phase| phase.duration));
    }
    let vtypes = read_verified(&verified, "scenario/vtypes.add.xml").expect("read profiles");
    for vtype in parse_vtypes_xml(&vtypes).expect("parse profiles") {
        values.extend([
            vtype.accel,
            vtype.decel,
            vtype.length,
            vtype.min_gap,
            vtype.max_speed,
        ]);
    }
    let mut departures = 0;
    for ordinal in 0..3 {
        let path = format!("scenario/DUERoutes/local.static.{ordinal}.rou.xml");
        let xml = read_verified(&verified, &path).expect("read departures");
        let vehicles =
            parse_due_routes_xml_filtered(&xml, ordinal, |_, _| false).expect("parse departures");
        departures += vehicles.len();
        values.extend(vehicles.into_iter().map(|vehicle| vehicle.depart));
    }
    assert_eq!(departures, 215_526);
    recheck_source_revision(root).expect("recheck source revision");
    values
}

#[test]
#[ignore = "需要 LUST_SOURCE_DIR 指向 pinned c4bd5bd3；可选 LUST_DECIMAL_BENCH=1 仅用于 release 局部测量"]
fn pinned_lust_decimal_rendering_matches_reference() {
    let source = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let values = pinned_lust_values(Path::new(&source));
    assert_eq!(values.len(), 1_342_577, "pinned corpus coverage");
    for &value in &values {
        assert_same_rendering(value);
    }
    eprintln!(
        "decimal-corpus: source={LUST_COMMIT} values={}",
        values.len()
    );
    if std::env::var("LUST_DECIMAL_BENCH").as_deref() == Ok("1") {
        #[cfg(debug_assertions)]
        panic!("benchmark requires release profile");
        #[cfg(not(debug_assertions))]
        benchmark_rendering(&values);
    }
}

#[cfg(not(debug_assertions))]
fn benchmark_rendering(values: &[ExactDecimal]) {
    use std::{hint::black_box, time::Instant};

    const REPEATS: usize = 2;
    let renderers: [fn(ExactDecimal) -> String; 2] =
        [reference_text, ExactDecimal::to_plain_string];
    let measure = |render: fn(ExactDecimal) -> String, parse: bool| {
        let start = Instant::now();
        let mut checksum = 0_u64;
        for _ in 0..REPEATS {
            for &value in values {
                let text = render(black_box(value));
                let output = if parse {
                    black_box(text.as_str())
                        .parse::<f64>()
                        .expect("parse")
                        .to_bits()
                } else {
                    black_box(text.as_str()).len() as u64
                };
                checksum = checksum.wrapping_add(black_box(output));
            }
        }
        (start.elapsed().as_secs_f64() * 1_000.0, black_box(checksum))
    };
    for parse in [false, true] {
        for render in renderers {
            black_box(measure(render, parse));
        }
        let mut samples = [Vec::new(), Vec::new()];
        let mut checksum = None;
        // 交错先后顺序，避免每轮固定把冷态或热态偏向同一实现。
        for round in 0..10 {
            let order = if round % 2 == 0 { [0, 1] } else { [1, 0] };
            for variant in order {
                let (elapsed, actual) = measure(renderers[variant], parse);
                if let Some(expected) = checksum {
                    assert_eq!(actual, expected, "benchmark checksum");
                } else {
                    checksum = Some(actual);
                }
                samples[variant].push(elapsed);
            }
        }
        eprintln!(
            "decimal-benchmark: {}",
            serde_json::json!({
                "source_commit": LUST_COMMIT,
                "values": values.len(),
                "repeats": REPEATS,
                "parse_binary64": parse,
                "reference_ms": samples[0],
                "candidate_ms": samples[1],
            })
        );
    }
}
