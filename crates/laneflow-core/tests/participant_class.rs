use laneflow_core::{
    CoreError, ParticipantClass, ParticipantClassHandle, ParticipantClassRegistry,
};

fn seed_classes() -> Vec<ParticipantClass> {
    vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
        ParticipantClass::new("bus", Some("motorVehicle")),
        ParticipantClass::new("largeBus", Some("bus")),
        ParticipantClass::new("nonMotor", None),
        ParticipantClass::new("bicycle", Some("nonMotor")),
        ParticipantClass::new("pedestrian", None),
    ]
}

fn class_handle(registry: &ParticipantClassRegistry, external_id: &str) -> ParticipantClassHandle {
    registry
        .class_handle(external_id)
        .expect("class handle must exist")
}

#[test]
fn registry_resolves_handles_and_external_ids_in_normalization_order() {
    let registry = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("b-class", None),
        ParticipantClass::new("a-class", None),
    ])
    .expect("valid class registry");

    assert!(!registry.is_empty());
    assert_eq!(registry.class_count(), 2);
    let ids = registry
        .classes()
        .map(|handle| {
            registry
                .class_external_id(handle)
                .expect("class external ID")
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, ["b-class", "a-class"]);

    let a_class = class_handle(&registry, "a-class");
    assert_eq!(
        registry.class(a_class).map(ParticipantClass::id),
        Some("a-class")
    );
    assert_eq!(registry.class_handle("missing"), None);
}

#[test]
fn class_id_syntax_error_precedes_duplicate_in_input_order() {
    let error = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("valid", None),
        ParticipantClass::new("bad id", None),
        ParticipantClass::new("valid", None),
    ])
    .expect_err("malformed class id must fail first");

    std::assert_matches!(
        error,
        CoreError::InvalidExternalId {
            field,
            external_id,
            ..
        } if field == "participantClasses[].id" && external_id == "bad id"
    );
}

#[test]
fn duplicate_class_id_is_rejected_in_input_order() {
    let error = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("car", None),
        ParticipantClass::new("bus", None),
        ParticipantClass::new("car", None),
    ])
    .expect_err("duplicate class id must fail");

    std::assert_matches!(
        error,
        CoreError::DuplicateParticipantClassId { class_id } if class_id == "car"
    );
}

#[test]
fn unknown_extends_id_is_rejected_with_attribution() {
    let error = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("missing")),
    ])
    .expect_err("unknown extends id must fail");

    std::assert_matches!(
        error,
        CoreError::UnknownParticipantClassExtends {
            class_id,
            extends_id,
        } if class_id == "car" && extends_id == "missing"
    );
}

#[test]
fn extends_id_uses_shared_ascii_token_rule() {
    let error =
        ParticipantClassRegistry::try_new(vec![ParticipantClass::new("car", Some("bad id"))])
            .expect_err("malformed extends id must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidExternalId {
            field,
            external_id,
            ..
        } if field == "participantClasses[].extendsId" && external_id == "bad id"
    );
}

#[test]
fn inheritance_cycle_is_rejected_with_attribution() {
    for (classes, expected_id) in [
        (
            vec![
                ParticipantClass::new("a", Some("b")),
                ParticipantClass::new("b", Some("a")),
            ],
            "a",
        ),
        (
            vec![ParticipantClass::new("selfish", Some("selfish"))],
            "selfish",
        ),
        (
            vec![
                ParticipantClass::new("root", None),
                ParticipantClass::new("a", Some("c")),
                ParticipantClass::new("b", Some("a")),
                ParticipantClass::new("c", Some("a")),
            ],
            "a",
        ),
    ] {
        let error =
            ParticipantClassRegistry::try_new(classes).expect_err("inheritance cycle must fail");
        std::assert_matches!(
            error,
            CoreError::ParticipantClassInheritanceCycle { class_id } if class_id == expected_id
        );
    }
}

#[test]
fn identity_error_precedes_unknown_extends_and_cycle() {
    let error = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("bad id", Some("missing")),
        ParticipantClass::new("a", Some("b")),
        ParticipantClass::new("b", Some("a")),
    ])
    .expect_err("identity error must fail first");
    std::assert_matches!(error, CoreError::InvalidExternalId { .. });

    let error = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("orphan", Some("missing")),
        ParticipantClass::new("a", Some("b")),
        ParticipantClass::new("b", Some("a")),
    ])
    .expect_err("unknown extends must fail before cycle detection");
    std::assert_matches!(
        error,
        CoreError::UnknownParticipantClassExtends { class_id, .. } if class_id == "orphan"
    );
}

#[test]
fn depth_reflects_inheritance_chain() {
    let registry = ParticipantClassRegistry::try_new(seed_classes()).expect("valid classes");

    assert_eq!(
        registry.depth(class_handle(&registry, "motorVehicle")),
        Some(0)
    );
    assert_eq!(registry.depth(class_handle(&registry, "car")), Some(1));
    assert_eq!(registry.depth(class_handle(&registry, "bus")), Some(1));
    assert_eq!(registry.depth(class_handle(&registry, "largeBus")), Some(2));
    assert_eq!(
        registry.depth(class_handle(&registry, "pedestrian")),
        Some(0)
    );
    assert_eq!(registry.depth(class_handle(&registry, "bicycle")), Some(1));
}

#[test]
fn descendant_query_covers_self_transitive_descendants_and_non_relatives() {
    let registry = ParticipantClassRegistry::try_new(seed_classes()).expect("valid classes");
    let motor_vehicle = class_handle(&registry, "motorVehicle");
    let car = class_handle(&registry, "car");
    let bus = class_handle(&registry, "bus");
    let large_bus = class_handle(&registry, "largeBus");
    let non_motor = class_handle(&registry, "nonMotor");
    let bicycle = class_handle(&registry, "bicycle");
    let pedestrian = class_handle(&registry, "pedestrian");

    // 自身匹配。
    for class in [
        motor_vehicle,
        car,
        bus,
        large_bus,
        non_motor,
        bicycle,
        pedestrian,
    ] {
        assert!(registry.is_descendant_or_self(class, class));
    }

    // 传递后代匹配。
    assert!(registry.is_descendant_or_self(car, motor_vehicle));
    assert!(registry.is_descendant_or_self(bus, motor_vehicle));
    assert!(registry.is_descendant_or_self(large_bus, motor_vehicle));
    assert!(registry.is_descendant_or_self(large_bus, bus));
    assert!(registry.is_descendant_or_self(bicycle, non_motor));

    // 祖先、兄弟与无关分支不匹配。
    assert!(!registry.is_descendant_or_self(motor_vehicle, car));
    assert!(!registry.is_descendant_or_self(bus, car));
    assert!(!registry.is_descendant_or_self(car, bus));
    assert!(!registry.is_descendant_or_self(bicycle, motor_vehicle));
    assert!(!registry.is_descendant_or_self(pedestrian, non_motor));
    assert!(!registry.is_descendant_or_self(car, pedestrian));
}

#[test]
fn empty_registry_is_valid() {
    for registry in [
        ParticipantClassRegistry::empty(),
        ParticipantClassRegistry::try_new(Vec::new()).expect("empty input is valid"),
    ] {
        assert!(registry.is_empty());
        assert_eq!(registry.class_count(), 0);
        assert_eq!(registry.classes().count(), 0);
        assert_eq!(registry.class_handle("motorVehicle"), None);
    }
}

#[test]
fn input_permutation_preserves_external_id_aligned_semantics() {
    let canonical = ParticipantClassRegistry::try_new(seed_classes()).expect("valid classes");
    let mut permuted_classes = seed_classes();
    permuted_classes.reverse();
    let permuted = ParticipantClassRegistry::try_new(permuted_classes).expect("valid classes");

    assert_eq!(canonical.class_count(), permuted.class_count());

    let external_ids = canonical
        .classes()
        .map(|handle| {
            canonical
                .class_external_id(handle)
                .expect("class external ID")
                .to_owned()
        })
        .collect::<Vec<_>>();

    // 同一 external ID 在两个 registry 中的 depth 语义一致。
    for external_id in &external_ids {
        assert_eq!(
            canonical.depth(class_handle(&canonical, external_id)),
            permuted.depth(class_handle(&permuted, external_id)),
            "depth mismatch for {external_id}"
        );
    }

    // 同一 (descendant, ancestor) external ID 对的层级匹配语义一致。
    for descendant in &external_ids {
        for ancestor in &external_ids {
            assert_eq!(
                canonical.is_descendant_or_self(
                    class_handle(&canonical, descendant),
                    class_handle(&canonical, ancestor),
                ),
                permuted.is_descendant_or_self(
                    class_handle(&permuted, descendant),
                    class_handle(&permuted, ancestor),
                ),
                "descendant semantics mismatch for ({descendant}, {ancestor})"
            );
        }
    }
}

#[test]
fn descendant_query_rejects_out_of_range_handles() {
    let registry = ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
    ])
    .expect("valid classes");
    let larger = ParticipantClassRegistry::try_new(seed_classes()).expect("valid classes");
    // index 6，超出 2 类 registry 的范围。
    let out_of_range = class_handle(&larger, "pedestrian");
    let motor_vehicle = class_handle(&registry, "motorVehicle");

    assert!(!registry.is_descendant_or_self(out_of_range, motor_vehicle));
    assert!(!registry.is_descendant_or_self(motor_vehicle, out_of_range));
    assert!(!registry.is_descendant_or_self(out_of_range, out_of_range));
    assert_eq!(registry.depth(out_of_range), None);
    assert_eq!(registry.class_external_id(out_of_range), None);
}
