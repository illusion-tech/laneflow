use super::*;
use laneflow_road_editing_wire::runtime;

fn wire_strings(values: &[&str]) -> Vec<u8> {
    let mut builder = runtime::FlatBufferBuilder::new();
    let offsets: Vec<_> = values
        .iter()
        .map(|value| builder.create_string(value))
        .collect();
    let vector = builder.create_vector(&offsets);
    builder.finish(vector, None);
    builder.finished_data().to_vec()
}

#[test]
fn sorted_duplicates_match_pairwise_spelling_and_namespace_rules() {
    let limits = CompileLimits::p100_initial_v1();
    let scratch = PreflightScratch::new(&limits, 0);
    let alphabet = ["a", "city::a", "other::a", "b", "", "::a", "a::b::c"];
    for size in 0..=4_u32 {
        for mut seed in 0..alphabet.len().pow(size) {
            let values: Vec<_> = (0..size)
                .map(|_| {
                    let value = alphabet[seed % alphabet.len()];
                    seed /= alphabet.len();
                    value
                })
                .collect();
            let mut duplicate_string = false;
            let mut duplicate_reference = false;
            for (index, left) in values.iter().enumerate() {
                for right in &values[index + 1..] {
                    duplicate_string |= left == right;
                    let left_parts = left.split_once("::").unwrap_or(("city", left));
                    let right_parts = right.split_once("::").unwrap_or(("city", right));
                    duplicate_reference |= left_parts == right_parts;
                }
            }
            let bytes = wire_strings(&values);
            let wire = runtime::root::<StringVector<'_>>(&bytes).unwrap();
            let spelling = ensure_unique_strings(wire, "keys", "source", &scratch);
            let references = ensure_unique_references(wire, "city", "keys", "source", &scratch);
            assert_eq!(spelling.is_err(), duplicate_string, "{values:?}");
            assert_eq!(references.is_err(), duplicate_reference, "{values:?}");
            for error in [spelling.err(), references.err()].into_iter().flatten() {
                assert_eq!(
                    format!("{error:?}"),
                    format!(
                        "{:?}",
                        semantic_error("keys", RoadEditingInputViolation::DuplicateValue, "source",)
                    )
                );
            }
        }
    }
}

#[test]
fn duplicate_witness_keeps_the_first_source_index() {
    let limits = CompileLimits::p100_initial_v1();
    let scratch = PreflightScratch::new(&limits, 0);
    for mut seed in 0..1_024 {
        let values: Vec<_> = (0..5)
            .map(|_| {
                let value = seed % 4;
                seed /= 4;
                value
            })
            .collect();
        let expected = values
            .iter()
            .enumerate()
            .find_map(|(index, left)| values[index + 1..].contains(left).then_some(index));
        assert_eq!(
            first_duplicate_index(values.iter(), |value| *value, &scratch).unwrap(),
            expected
        );
    }
}

#[test]
fn scratch_budget_precedes_duplicate_only_when_the_index_cannot_fit() {
    let limits = CompileLimits::p100_initial_v1()
        .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 0);
    let scratch = PreflightScratch::new(&limits, 0);
    let bytes = wire_strings(&["a", "a"]);
    let wire = runtime::root::<StringVector<'_>>(&bytes).unwrap();
    let error = ensure_unique_strings(wire, "keys", "source", &scratch).unwrap_err();
    assert!(matches!(
        error.diagnostics()[0].payload(),
        crate::DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::StageScratchBytes,
            ..
        }
    ));
    let exact = limits.with_test_admission_limit(
        CompileLimitDimension::StageScratchBytes,
        (2 * size_of::<&str>()) as u32,
    );
    let scratch = PreflightScratch::new(&exact, 0);
    let error = ensure_unique_strings(wire, "keys", "source", &scratch).unwrap_err();
    assert!(matches!(
        error.diagnostics()[0].payload(),
        crate::DiagnosticPayload::InvalidRoadEditingSource {
            violation: RoadEditingSourceViolation::InvalidSemanticValue(
                RoadEditingInputViolation::DuplicateValue
            ),
            ..
        }
    ));
}

#[test]
fn preflight_peak_has_an_exact_boundary_and_does_not_include_external_source_bytes() {
    let limits = CompileLimits::single_network_1m_v2();
    let buffer = benchmark::corridor_source(4, &limits);
    let input = super::super::RoadEditingModuleInput::try_new(
        "preflight-benchmark",
        buffer.as_bytes(),
        None,
    )
    .unwrap();
    let verified = super::super::reader::verify_source(input, &limits, 0, 0, 0).unwrap();
    let peak = verified.preflight_counts().preflight_peak_scratch_bytes();
    assert!(peak > 0 && peak < buffer.as_bytes().len() as u64);
    let exact = limits
        .clone()
        .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, peak as u32);
    let exact_result = super::super::reader::verify_source(input, &exact, 0, 0, 0).unwrap();
    assert_eq!(verified.preflight_counts(), exact_result.preflight_counts());
    let short = exact
        .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, (peak - 1) as u32);
    assert!(super::super::reader::verify_source(input, &short, 0, 0, 0).is_err());
    let live = limits.with_test_admission_limit(
        CompileLimitDimension::CompilerControlledLiveBytes,
        (peak + 100) as u32,
    );
    assert!(super::super::reader::verify_source(input, &live, 0, 0, 100).is_ok());
    assert!(super::super::reader::verify_source(input, &live, 0, 0, 101).is_err());
}
