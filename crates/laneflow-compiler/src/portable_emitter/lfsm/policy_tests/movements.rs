use super::*;

fn movement_indices(map: &OwnedObject) -> Vec<usize> {
    map.sections[2].tables[0]
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| (row.fields[0].value == OwnedValue::U16(6)).then_some(i))
        .collect()
}

// 保持位置池完整，确保负例不能仅因被删来源的位置失去引用而遭到拒绝。
fn retain_locations(map: &mut OwnedObject, removed: &OwnedRow) {
    let mut removed = removed.clone();
    let OwnedValue::U32(primary) = *value(&mut removed, 4) else {
        panic!()
    };
    let OwnedValue::OrdinalVectorU32(contributions) = value(&mut removed, 5) else {
        panic!()
    };
    let row = &mut map.sections[3].tables[0].rows[0];
    assert_ne!(row.fields[0].value, OwnedValue::U16(24));
    let OwnedValue::OrdinalVectorU32(existing) = value(row, 6) else {
        panic!()
    };
    let mut locations = existing.to_vec();
    locations.push(primary);
    locations.extend_from_slice(contributions);
    locations.sort_unstable();
    locations.dedup();
    *existing = locations.into();
}

#[test]
fn movement_sources_reject_duplicates_with_hidden_direction_contributions() {
    for fixture in [
        empty_policy_fixture(),
        fixture(false, None, 2),
        fixture(true, Some("node"), 2),
    ] {
        fixture.check(&fixture.map).unwrap();
        let index = movement_indices(&fixture.map)[0];
        for forged in [false, true] {
            for offset in [0, 1] {
                let mut map = fixture.map.clone();
                let mut rows = map.sections[2].tables[0].rows.to_vec();
                let mut extra = rows[index].clone();
                if forged {
                    *value(&mut extra, 5) = OwnedValue::OrdinalVectorU32([0, 1].into());
                }
                rows.insert(index + offset, extra);
                map.sections[2].tables[0].rows = rows.into();
                assert_eq!(
                    fixture.check(&map),
                    Err(PortableEmissionError::PolicySourceMismatch),
                    "forged={forged}, offset={offset}"
                );
            }
        }
        fixture.check(&fixture.map).unwrap();
    }
}

#[test]
fn movement_sources_require_exact_counts_and_unique_ids_on_both_paths() {
    for fixture in [
        empty_policy_fixture(),
        fixture(false, None, 2),
        fixture(true, None, 2),
    ] {
        let indices = movement_indices(&fixture.map);
        assert!(indices.len() >= 2);
        let first = indices[0];
        let second = indices[1];
        let last = *indices.last().unwrap();
        let mut control = fixture.map.clone();
        retain_locations(
            &mut control,
            &fixture.map.sections[2].tables[0].rows[second],
        );
        fixture.check(&control).unwrap();
        for mutation in 0..4 {
            let mut map = control.clone();
            let mut rows = map.sections[2].tables[0].rows.to_vec();
            match mutation {
                0 => {
                    rows.remove(second);
                }
                1 => {
                    let mut extra = rows[last].clone();
                    *value(&mut extra, 2) = OwnedValue::StableId128([255; 16]);
                    *value(&mut extra, 3) = OwnedValue::U32(indices.len() as u32);
                    rows.insert(last + 1, extra);
                }
                2 => {
                    // 总行数和 ordinal 都不变，只把第二个身份替换成重复的第一个身份。
                    let id = value(&mut rows[first], 2).clone();
                    *value(&mut rows[second], 2) = id;
                }
                3 => rows.swap(first, second),
                _ => unreachable!(),
            }
            map.sections[2].tables[0].rows = rows.into();
            assert_eq!(
                fixture.check(&map),
                Err(PortableEmissionError::PolicySourceMismatch),
                "mutation {mutation}"
            );
        }
        fixture.check(&fixture.map).unwrap();
    }
}

#[test]
fn excess_movement_sources_across_chunks_fail_in_the_empty_policy_path() {
    let fixture = empty_policy_fixture();
    let mut map = fixture.map.clone();
    let indices = movement_indices(&map);
    for &index in &indices {
        retain_locations(&mut map, &fixture.map.sections[2].tables[0].rows[index]);
    }
    fixture.check(&map).unwrap();
    let template = map.sections[2].tables[0].rows[indices[0]].clone();
    let mut rows = map.sections[2].tables[0].rows.to_vec();
    let extra = (0..65_537_u32).map(|ordinal| {
        let mut row = template.clone();
        let mut id = [0; 16];
        id[12..].copy_from_slice(&ordinal.to_be_bytes());
        *value(&mut row, 2) = OwnedValue::StableId128(id);
        *value(&mut row, 3) = OwnedValue::U32(ordinal);
        row
    });
    rows.splice(indices[0]..=*indices.last().unwrap(), extra);
    map.sections[2].tables[0].rows = rows.into();
    let map = bytes(&map);
    let view = preflight_object_values(&map, PortableObjectKind::SourceMap, FormatLimits::HARD)
        .unwrap()
        .registry_view();
    assert!(view.section(2).unwrap().table(0).unwrap().chunk_count() >= 2);
    assert_eq!(
        check_portable_policy_sources(
            fixture.artifact.bytes(),
            fixture.output.source_map_input(),
            &map,
            FormatLimits::HARD,
            &crate::CompileLimits::p100_initial_v1(),
        ),
        Err(PortableEmissionError::PolicySourceMismatch)
    );
    fixture.check(&fixture.map).unwrap();
}
