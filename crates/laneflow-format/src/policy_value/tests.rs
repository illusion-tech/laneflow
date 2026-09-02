use super::*;
use crate::FormatLimitConfig;
use laneflow_static_contract::{PortableObjectKind, portable_object_schema};
use std::vec::Vec;

fn field(tag: u16, ty: u8, value: &[u8]) -> Vec<u8> {
    [
        tag.to_le_bytes().as_slice(),
        &[ty, 0],
        &(value.len() as u64).to_le_bytes(),
        value,
    ]
    .concat()
}
fn row(fields: &[Vec<u8>]) -> Vec<u8> {
    let length = 16 + fields.iter().map(Vec::len).sum::<usize>();
    [
        &(length as u64).to_le_bytes()[..],
        &(fields.len() as u32).to_le_bytes(),
        &[0; 4],
        &fields.concat(),
    ]
    .concat()
}
fn stable(kind: u16, id: u8) -> Vec<u8> {
    [kind.to_le_bytes().as_slice(), &[id; 16]].concat()
}
fn vector(kind: u16, ids: &[u8]) -> Vec<u8> {
    [
        (ids.len() as u32).to_le_bytes().as_slice(),
        &ids.iter()
            .flat_map(|id| stable(kind, *id))
            .collect::<Vec<_>>(),
    ]
    .concat()
}
fn stream() -> Vec<u8> {
    row(&[
        field(3, 10, &stable(23, 1)),
        field(4, 10, &vector(18, &[2])),
        field(5, 13, &0_i32.to_le_bytes()),
        field(6, 10, &vector(23, &[3])),
        field(7, 9, b"g"),
        field(
            8,
            12,
            &[&1_u32.to_le_bytes()[..], &row(&[field(1, 9, b"e")])].concat(),
        ),
    ])
}
fn change(kind: u8, before: &[u8], after: &[u8]) -> Vec<u8> {
    row(&[
        field(1, 1, &[2]),
        field(2, 7, &[1; 16]),
        field(3, 1, &[kind]),
        field(4, 9, b"k"),
        field(5, 10, before),
        field(6, 10, after),
    ])
}
fn check(bytes: &[u8], limits: FormatLimits) -> Result<PreflightBudget, FormatError> {
    let schema = portable_object_schema(PortableObjectKind::SemanticDiff).sections[6].tables[0].row;
    let mut budget = PreflightBudget::default();
    preflight_embedded_row(bytes, schema, limits, &mut budget)?;
    Ok(budget)
}

#[test]
fn embedded_before_and_after_share_outer_string_and_vector_budgets() {
    let value = stream();
    let bytes = change(2, &value, &value);
    let budget = check(&bytes, FormatLimits::HARD).unwrap();
    assert_eq!(budget.total_utf8_bytes, 5); // outer key + 2 * (gap + evidence key)
    assert_eq!(budget.total_vector_bytes, 154); // 2 * (22 class + 22 yield + 33 evidence)
    let mut config = FormatLimitConfig::HARD;
    config.max_total_utf8_bytes = 5;
    config.max_total_vector_bytes = 154;
    check(&bytes, FormatLimits::try_new(config).unwrap()).unwrap();
    config.max_total_vector_bytes = 153;
    assert!(matches!(
        check(&bytes, FormatLimits::try_new(config).unwrap()),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::TotalVectorBytes,
            ..
        })
    ));
    config.max_total_vector_bytes = 154;
    config.max_total_utf8_bytes = 4;
    assert!(matches!(
        check(&bytes, FormatLimits::try_new(config).unwrap()),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::TotalUtf8Bytes,
            ..
        })
    ));
    check(&bytes, FormatLimits::HARD).unwrap();
}

#[test]
fn embedded_row_and_stable_reference_shapes_are_closed() {
    let good = stream();
    for mutation in 0..8 {
        let mut bad = good.clone();
        match mutation {
            0 => bad.clear(),
            1 => bad.push(0),
            2 => bad[12] = 1,
            3 => bad[18] = 3, // stream must be Bytes, not U32
            4 => bad[28..30].copy_from_slice(&18_u16.to_le_bytes()), // wrong target kind
            5 => bad[58..62].copy_from_slice(&u32::MAX.to_le_bytes()), // checked vector count
            6 => bad[16..18].copy_from_slice(&2_u16.to_le_bytes()), // owner cannot appear in payload
            7 => bad[8..12].copy_from_slice(&100_u32.to_le_bytes()),
            _ => unreachable!(),
        }
        assert!(
            check(&change(2, &bad, &good), FormatLimits::HARD).is_err(),
            "mutation {mutation}"
        );
    }
    for refs in [vector(18, &[2, 2]), vector(18, &[3, 2]), vector(23, &[2])] {
        let bad = row(&[
            field(3, 10, &stable(23, 1)),
            field(4, 10, &refs),
            field(5, 13, &0_i32.to_le_bytes()),
            field(6, 10, &vector(23, &[])),
            field(8, 12, &0_u32.to_le_bytes()),
        ]);
        assert!(check(&change(2, &good, &bad), FormatLimits::HARD).is_err());
    }
    check(&change(2, &good, &good), FormatLimits::HARD).unwrap(); // same-value Modify is rejected by the value layer
}

#[test]
fn outer_change_shape_requires_exact_payload_sides() {
    let payload = row(&[field(3, 9, b"source")]);
    for (op, sides) in [(0, &[6_u16][..]), (1, &[5_u16][..]), (2, &[5_u16, 6][..])] {
        let mut fields = std::vec![
            field(1, 1, &[op]),
            field(2, 7, &[1; 16]),
            field(3, 1, &[0]),
            field(4, 9, b"k")
        ];
        fields.extend(sides.iter().map(|tag| field(*tag, 10, &payload)));
        check(&row(&fields), FormatLimits::HARD).unwrap();
        for omitted in 0..fields.len() {
            let mut bad = fields.clone();
            bad.remove(omitted);
            assert!(check(&row(&bad), FormatLimits::HARD).is_err());
        }
        let mut wrong = fields.clone();
        wrong[0] = field(1, 1, &[3]);
        assert!(check(&row(&wrong), FormatLimits::HARD).is_err());
        if op < 2 {
            let forbidden = if op == 0 { 5 } else { 6 };
            let mut wrong = fields.clone();
            wrong.push(field(forbidden, 10, &payload));
            wrong.sort_by_key(|v| u16::from_le_bytes([v[0], v[1]]));
            assert!(check(&row(&wrong), FormatLimits::HARD).is_err());
        }
    }
}
