//! Fixed #545 capacity-only input. The canonical #542 base is never rewritten.

use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use laneflow_compiler::{PortableDiffBase, PortableEmissionProvenance, emit_portable_candidate};
use laneflow_format::{FormatLimits, check_post_emission_bundle, preflight_object_values};
use laneflow_runtime::SemanticDiffOriginBinding;
use laneflow_static_contract::{
    EntityKind, ExactByteLength, ParkingFacilityId, ParkingFacilityOrdinal, PortableObjectKind,
    SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use laneflow_urban_generator::{
    Scale, UrbanConfig, compile_source, generate_capacity_increment_source, generate_source,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    Artifacts, Result, checked, invalid,
    report::{digest_file, write_json},
};

pub fn prepare_capacity_variant(base_directory: &Path, output: &Path) -> Result<Value> {
    let base = Artifacts::load_spatial(base_directory)?;
    let identity = base.revision().identity();
    let mut candidates = Vec::new();
    for raw in 0..identity.entity_count(EntityKind::ParkingFacility) {
        let ordinal = ParkingFacilityOrdinal::from_raw(raw);
        let facility = base
            .revision()
            .traffic()
            .relations()
            .parking_facility(ordinal)
            .ok_or_else(|| invalid("missing canonical facility"))?;
        if facility.spaces().is_empty() && (1..u32::MAX).contains(&facility.virtual_capacity()) {
            let id = identity
                .stable_id(ordinal)
                .ok_or_else(|| invalid("missing facility identity"))?;
            candidates.push((
                id.into_untyped().into_bytes(),
                id,
                facility.virtual_capacity(),
            ));
        }
    }
    candidates.sort_unstable_by_key(|(bytes, _, _)| *bytes);
    let (_, selected, capacity) = candidates
        .first()
        .copied()
        .ok_or_else(|| invalid("no eligible virtual-only facility"))?;
    let selected_hex = format!("{:x}", selected.into_untyped());
    let catalog_target = base
        .catalog()
        .parking
        .iter()
        .find(|p| p.kind == "virtual" && p.facility_stable_id == selected_hex)
        .ok_or_else(|| invalid("selected canonical facility absent from source catalog"))?;
    let config = checked(
        "urban configuration",
        UrbanConfig::parse(&fs::read_to_string(base_directory.join("config.toml"))?),
    )?;
    let scale = checked("urban scale", Scale::parse(&base.catalog().scale))?;
    let ordinary = checked("regenerate base source", generate_source(&config, scale))?;
    if ordinary.common.as_bytes() != fs::read(base_directory.join("common.lfre"))?
        || ordinary.topology.as_bytes() != fs::read(base_directory.join("topology.lfre"))?
    {
        return Err(invalid(
            "base source is not reproducible from the frozen generator input",
        ));
    }
    let variant = checked(
        "capacity increment source",
        generate_capacity_increment_source(&config, scale, &catalog_target.facility),
    )?;
    if ordinary.common.as_bytes() != variant.common.as_bytes()
        || serde_json::to_value(&ordinary.edges)? != serde_json::to_value(&variant.edges)?
        || serde_json::to_value(&ordinary.movements)? != serde_json::to_value(&variant.movements)?
        || serde_json::to_value(&ordinary.signals)? != serde_json::to_value(&variant.signals)?
    {
        return Err(invalid("capacity variant changed unrelated source inputs"));
    }
    let mut expected_parking = serde_json::to_value(&ordinary.parking)?;
    for pool in expected_parking.as_array_mut().expect("array") {
        if pool["key"] == catalog_target.key {
            pool["capacity"] = json!(capacity + 1);
        }
    }
    if expected_parking != serde_json::to_value(&variant.parking)? {
        return Err(invalid("capacity variant changed other parking semantics"));
    }
    let compiled = checked("compile capacity variant", compile_source(&variant))?;
    let base_bytes = fs::read(base_directory.join("network.lfca"))?;
    let base_view = checked(
        "base LFCA values",
        preflight_object_values(
            &base_bytes,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        ),
    )?;
    let provenance = checked(
        "variant provenance",
        PortableEmissionProvenance::try_new("laneflow-urban-capacity-increment-v1"),
    )?;
    let candidate = checked(
        "emit capacity variant",
        emit_portable_candidate(
            &compiled,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Artifact(base_view),
        ),
    )?;
    let checked_bundle = checked(
        "check capacity variant",
        check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        ),
    )?;
    let target = checked(
        "build capacity variant",
        build_shared_network_revision(
            checked_bundle.canonical_network_input(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(2_147_483_648, 2_147_483_648),
            ),
        ),
    )?;
    verify_capacity_change(base.revision(), &target, selected)?;
    fs::create_dir(output)?;
    for (name, bytes) in [
        ("common.lfre", variant.common.as_bytes()),
        ("topology.lfre", variant.topology.as_bytes()),
        ("network.lfca", candidate.canonical_artifact().bytes()),
        ("source-map.lfsm", candidate.source_map().bytes()),
        ("base-to-target.lfsd", candidate.semantic_diff().bytes()),
    ] {
        fs::write(output.join(name), bytes)?;
    }
    // #538：变体目录同时是可加载的完整工件目录——catalog 与 manifest 按目标修订
    // 和实际写入字节重建，供 gate 的未切换变体基线直接 `Artifacts::load_spatial`
    // 安装；完成后自检加载（也是对重建一致性的强校验）。
    // 边界：不承诺链式构造（生成器只产 ordinary+1 的增量源，可复现性校验按
    // ordinary 固定输入，对变体目录再跑 `variant` 会在复现检查处被拒）；也不作
    // 发布工件包——genesis.lfsd 仍绑定基座制品、manifest 的编译统计字段仍是
    // 基座描述值，仅 harness 加载与测量使用。
    let digest_entry = |bytes: &[u8]| -> toml::value::Table {
        let mut entry = toml::value::Table::new();
        entry.insert("bytes".into(), toml::Value::Integer(bytes.len() as i64));
        entry.insert("sha256".into(), toml::Value::String(crate::sha256(bytes)));
        entry
    };
    let target_revision = format!("{:x}", target.network_revision().as_digest());
    let mut catalog: toml::Value =
        toml::from_str(&fs::read_to_string(base_directory.join("routes.toml"))?)
            .map_err(|error| invalid(format!("base catalog parse: {error}")))?;
    {
        let table = catalog
            .as_table_mut()
            .ok_or_else(|| invalid("base catalog is not a table"))?;
        table.insert(
            "network_revision".into(),
            toml::Value::String(target_revision.clone()),
        );
        let pools = table
            .get_mut("parking")
            .and_then(|parking| parking.as_array_mut())
            .ok_or_else(|| invalid("base catalog has no parking table"))?;
        let mut updated = false;
        for pool in pools {
            let matched = pool
                .get("key")
                .and_then(|key| key.as_str())
                .map(|key| key == catalog_target.key)
                .unwrap_or(false);
            if matched {
                let entry = pool
                    .as_table_mut()
                    .ok_or_else(|| invalid("catalog parking entry is not a table"))?;
                match entry.get_mut("capacity") {
                    Some(value) => *value = toml::Value::Integer(i64::from(capacity + 1)),
                    None => return Err(invalid("catalog parking entry has no capacity")),
                }
                updated = true;
            }
        }
        if !updated {
            return Err(invalid("selected facility absent from rebuilt catalog"));
        }
    }
    let routes_bytes = toml::to_string_pretty(&catalog)?;
    fs::write(output.join("routes.toml"), &routes_bytes)?;
    for name in ["config.toml", "genesis.lfsd", "measurements.toml"] {
        fs::copy(base_directory.join(name), output.join(name))?;
    }
    let mut manifest: toml::Value =
        toml::from_str(&fs::read_to_string(base_directory.join("manifest.toml"))?)
            .map_err(|error| invalid(format!("base manifest parse: {error}")))?;
    {
        let table = manifest
            .as_table_mut()
            .ok_or_else(|| invalid("base manifest is not a table"))?;
        table.insert(
            "network_revision".into(),
            toml::Value::String(target_revision.clone()),
        );
        let files = table
            .get_mut("files")
            .and_then(|files| files.as_table_mut())
            .ok_or_else(|| invalid("base manifest has no files table"))?;
        files.insert(
            "network.lfca".into(),
            toml::Value::Table(digest_entry(candidate.canonical_artifact().bytes())),
        );
        files.insert(
            "source-map.lfsm".into(),
            toml::Value::Table(digest_entry(candidate.source_map().bytes())),
        );
        files.insert(
            "routes.toml".into(),
            toml::Value::Table(digest_entry(routes_bytes.as_bytes())),
        );
        // 容量增量合法改变 topology（parking 表住在其中）；common 经逐字节校验相等。
        // manifest 条目一律按实际写入字节重建，不沿用基座摘要。
        files.insert(
            "topology.lfre".into(),
            toml::Value::Table(digest_entry(variant.topology.as_bytes())),
        );
        files.insert(
            "common.lfre".into(),
            toml::Value::Table(digest_entry(variant.common.as_bytes())),
        );
    }
    fs::write(
        output.join("manifest.toml"),
        toml::to_string_pretty(&manifest)?,
    )?;
    let completed = Artifacts::load_spatial(output)?;
    if completed.catalog().network_revision != target_revision
        || completed.revision().network_revision() != target.network_revision()
    {
        return Err(invalid("completed variant directory identity mismatch"));
    }
    let mut files = BTreeMap::new();
    for name in [
        "common.lfre",
        "topology.lfre",
        "network.lfca",
        "source-map.lfsm",
        "base-to-target.lfsd",
    ] {
        files.insert(name, digest_file(&output.join(name))?);
    }
    let result = json!({"version":"urban-capacity-variant-v2","scale":base.catalog().scale,
        "bundle_scope":"harness-loadable measurement artifacts; not chainable (generator is ordinary+1 only); not a publication bundle (genesis binding and compile statistics remain base-descriptive)",
        "facility_key":catalog_target.facility,"facility_stable_id":selected_hex,
        "base_capacity":capacity,"target_capacity":capacity+1,
        "base_lfca":crate::artifacts::artifact_file_digest(&base_directory.join("network.lfca"))?,
        "base_network_revision":base.catalog().network_revision,
        "target_network_revision":format!("{:x}", target.network_revision().as_digest()),
        "source_delta":"one virtual-only facility capacity +1; no road, geometry, profile, signal, parking anchor or identity changes",
        "files":files});
    write_json(&output.join("variant.json"), &result)?;
    Ok(result)
}

pub(crate) fn verify_capacity_change(
    base: &SharedNetworkRevision,
    target: &SharedNetworkRevision,
    selected: ParkingFacilityId,
) -> Result<()> {
    if first_eligible(base)? != selected || base.network_revision() == target.network_revision() {
        return Err(invalid(
            "capacity variant did not increment the first eligible facility",
        ));
    }
    for kind in EntityKind::ALL {
        if base.identity().entity_count(kind) != target.identity().entity_count(kind) {
            return Err(invalid("capacity variant changed entity counts"));
        }
    }
    for raw in 0..base.identity().entity_count(EntityKind::ParkingFacility) {
        let ordinal = ParkingFacilityOrdinal::from_raw(raw);
        let id = base
            .identity()
            .stable_id(ordinal)
            .ok_or_else(|| invalid("base facility identity absent"))?;
        let target_ordinal = target
            .identity()
            .ordinal(id)
            .ok_or_else(|| invalid("target facility identity absent"))?;
        if ordinal != target_ordinal {
            return Err(invalid("capacity variant reordered facilities"));
        }
        let before = base
            .traffic()
            .relations()
            .parking_facility(ordinal)
            .ok_or_else(|| invalid("base facility absent"))?;
        let after = target
            .traffic()
            .relations()
            .parking_facility(target_ordinal)
            .ok_or_else(|| invalid("target facility absent"))?;
        if after.virtual_capacity() != before.virtual_capacity() + u32::from(id == selected)
            || before.spaces() != after.spaces()
            || before.virtual_entries() != after.virtual_entries()
            || before.virtual_exits() != after.virtual_exits()
        {
            return Err(invalid("unexpected canonical parking delta"));
        }
    }
    Ok(())
}

pub(crate) struct CapacityVariant {
    pub(crate) root: Arc<SharedNetworkRevision>,
    pub(crate) diff: Vec<u8>,
    pub(crate) binding: SemanticDiffOriginBinding,
    pub(crate) selected: ParkingFacilityId,
}

pub(crate) fn load_variant(base: &Artifacts, directory: &Path) -> Result<CapacityVariant> {
    let manifest: Value = serde_json::from_slice(&fs::read(directory.join("variant.json"))?)?;
    if manifest["version"] != "urban-capacity-variant-v2"
        || manifest["base_network_revision"] != base.catalog().network_revision
    {
        return Err(invalid("capacity variant base mismatch"));
    }
    if manifest["base_lfca"] != serde_json::to_value(&base.files["network.lfca"])?
        || manifest["scale"] != base.catalog().scale
        || manifest["files"].as_object().map(|v| v.len()) != Some(5)
    {
        return Err(invalid("capacity variant input manifest mismatch"));
    }
    for (name, digest) in manifest["files"]
        .as_object()
        .ok_or_else(|| invalid("variant files absent"))?
    {
        if !matches!(
            name.as_str(),
            "common.lfre"
                | "topology.lfre"
                | "network.lfca"
                | "source-map.lfsm"
                | "base-to-target.lfsd"
        ) {
            return Err(invalid("unknown variant input"));
        }
        if serde_json::to_value(digest_file(&directory.join(name))?)? != *digest {
            return Err(invalid("capacity variant digest mismatch"));
        }
    }
    let bytes = fs::read(directory.join("network.lfca"))?;
    let input = checked(
        "target LFCA",
        laneflow_format::check_canonical_network_input(bytes.as_slice(), FormatLimits::HARD),
    )?;
    let root = checked(
        "target shared network",
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(2_147_483_648, 2_147_483_648),
            ),
        ),
    )?;
    let selected = crate::catalog_id(
        manifest["facility_stable_id"]
            .as_str()
            .ok_or_else(|| invalid("variant facility missing"))?,
    )?;
    verify_capacity_change(base.revision(), &root, selected)?;
    let facility = base
        .revision()
        .identity()
        .ordinal(selected)
        .ok_or_else(|| invalid("selected facility absent"))?;
    let capacity = base
        .revision()
        .traffic()
        .relations()
        .parking_facility(facility)
        .ok_or_else(|| invalid("selected facility absent"))?
        .virtual_capacity();
    if manifest["base_capacity"] != capacity
        || manifest["target_capacity"] != capacity + 1
        || manifest["target_network_revision"]
            != format!("{:x}", root.network_revision().as_digest())
    {
        return Err(invalid("capacity variant semantic manifest mismatch"));
    }
    let diff = fs::read(directory.join("base-to-target.lfsd"))?;
    let binding = SemanticDiffOriginBinding::new(
        SEMANTIC_DIFF_FORMAT_VERSION,
        Sha256Digest::from_bytes(Sha256::digest(&diff).into()),
        ExactByteLength::new(diff.len() as u64),
    );
    Ok(CapacityVariant {
        root,
        diff,
        binding,
        selected,
    })
}

fn first_eligible(base: &SharedNetworkRevision) -> Result<ParkingFacilityId> {
    let mut ids = Vec::new();
    for raw in 0..base.identity().entity_count(EntityKind::ParkingFacility) {
        let ordinal = ParkingFacilityOrdinal::from_raw(raw);
        let facility = base
            .traffic()
            .relations()
            .parking_facility(ordinal)
            .ok_or_else(|| invalid("missing facility"))?;
        if facility.spaces().is_empty() && (1..u32::MAX).contains(&facility.virtual_capacity()) {
            ids.push(
                base.identity()
                    .stable_id(ordinal)
                    .ok_or_else(|| invalid("missing facility identity"))?,
            );
        }
    }
    ids.into_iter()
        .min_by_key(|id| id.into_untyped().into_bytes())
        .ok_or_else(|| invalid("no eligible facility"))
}
