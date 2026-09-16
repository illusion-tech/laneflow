//! Finite replay and capacity-only migration witnesses using the shared demand program.

use std::{
    collections::HashMap,
    fs,
    io::{BufWriter, Write},
    path::Path,
    sync::Arc,
    time::Instant,
};

use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowCommittedPoseBatch, LaneFlowSession, LaneFlowTargetSpatial,
};
use laneflow_runtime::*;
use laneflow_spatial::{FramePlacementToken, SpatialSession};
use laneflow_static_network::SharedNetworkRevision;
use serde_json::{Value, json};

use crate::{
    Artifacts, Harness, Presentation, ResolvedPlan, Result, UrbanCase, Window, checked,
    host::Host,
    invalid,
    report::{digest_file, write_json},
    variant::{CapacityVariant, load_variant},
};

const SAVE_TICK: u64 = 32;
const COMMIT_TICK: u64 = 8;
const END_TICK: u64 = 160;
const PREFLIGHT: CutoverPreflightLimits = CutoverPreflightLimits::new(2_147_483_648);
const TRANSACTION: CutoverTransactionLimits = CutoverTransactionLimits {
    max_journal_bytes: 128 * 1_024 * 1_024,
    max_catch_up_lag_ticks: 8,
    max_records_per_pump: 4_096,
};

fn spatial(root: &Arc<SharedNetworkRevision>) -> Result<SpatialSession> {
    checked(
        "transition Spatial bind",
        SpatialSession::bind(root.clone()),
    )?
    .ok_or_else(|| invalid("transition geometry absent"))
}

fn target_source(root: &SharedNetworkRevision) -> Result<CommittedNetworkSource> {
    let origin = root.canonical_origin();
    Ok(CommittedNetworkSource::Published {
        reference: checked(
            "target source",
            PublishedLfcaReference::new(
                "urban://capacity-increment-v1",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            ),
        )?,
    })
}

/// Rebind caller identities through snapshot-local IDs. Runtime handles are never
/// encoded as durable identities, nor inferred from possibly identical routes/poses.
fn restore<'a>(mut harness: Harness<'a>, output: &Path) -> Result<Harness<'a>> {
    let snapshot = checked("capture replay save", harness.world.capture_snapshot())?;
    let old_routes: Vec<_> = harness.world.live_routes().collect();
    let old_vehicles = harness.world.live_vehicles().to_vec();
    if old_routes.len() != snapshot.routes().len()
        || old_vehicles.len() != snapshot.live_order().len()
    {
        return Err(invalid("snapshot identity cardinalities differ"));
    }
    let bytes = encode_lfrs(&snapshot);
    fs::write(output.join("save.lfrs"), &bytes)?;
    let restored = checked(
        "restore replay save",
        restore_lfrs(
            &bytes,
            harness.world.revision().clone(),
            harness.world.committed_source().clone(),
            harness.world.config(),
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(2_147_483_648, 4_096),
        ),
    )?;
    let mut routes = HashMap::new();
    for (old, captured) in old_routes.iter().zip(snapshot.routes()) {
        routes.insert(
            *old,
            restored
                .route_handle(captured.snapshot_route_id())
                .ok_or_else(|| invalid("restored route absent"))?,
        );
    }
    let mut vehicles = HashMap::new();
    let mut identity_map = Vec::new();
    for (old, snapshot_id) in old_vehicles.iter().zip(snapshot.live_order()) {
        let new = restored
            .vehicle_handle(*snapshot_id)
            .ok_or_else(|| invalid("restored vehicle absent"))?;
        vehicles.insert(*old, new);
        identity_map.push(
            json!({"individual":harness.stable_individual(*old),"snapshot_vehicle_id":snapshot_id}),
        );
    }
    write_json(
        &output.join("save-identities.json"),
        &json!({"version":1,"vehicles":identity_map,
        "routes":old_routes.iter().zip(snapshot.routes()).map(|(h,s)| json!({"route_key":harness.route_keys[h],"snapshot_route_id":s.snapshot_route_id()})).collect::<Vec<_>>()}),
    )?;
    for handle in harness.routes.values_mut() {
        *handle = routes[handle];
    }
    harness.route_keys = harness
        .route_keys
        .into_iter()
        .map(|(h, k)| (routes[&h], k))
        .collect();
    harness.route_edges = harness
        .route_edges
        .into_iter()
        .map(|(h, e)| (routes[&h], e))
        .collect();
    harness.slots.clear();
    for (slot, individual) in harness.individuals.iter_mut().enumerate() {
        if let Some(old) = individual.handle {
            let new = *vehicles
                .get(&old)
                .ok_or_else(|| invalid("caller identity absent from snapshot"))?;
            individual.handle = Some(new);
            harness.slots.insert(new, slot);
        }
    }
    let root = restored.world().revision().clone();
    let digest = harness.checkpoint()?;
    harness.world =
        Host::Headless(Box::new(restored.into_world())).into_adapter(spatial(&root)?)?;
    harness.step_before.clear();
    if harness.checkpoint()? != digest {
        return Err(invalid("save/restore digest differs"));
    }
    Ok(harness)
}

fn native<'h>(harness: &'h mut Harness<'_>) -> Result<&'h mut TrafficWorld> {
    match &mut harness.world {
        Host::Headless(world) => Ok(world),
        Host::Adapter(_) => Err(invalid("online witness requires native host")),
    }
}

fn selected_counts(harness: &Harness<'_>, variant: &CapacityVariant) -> Result<Value> {
    let ordinal = harness
        .world
        .revision()
        .identity()
        .ordinal(variant.selected)
        .ok_or_else(|| invalid("selected facility not bound"))?;
    let target = ParkingTarget::VirtualPool(ordinal);
    let mut occupied = 0;
    let mut reserved = 0;
    for handle in harness.world.live_vehicles() {
        match harness.world.parking_binding(*handle) {
            Some(ParkingBinding::Occupied(t)) if t == target => occupied += 1,
            Some(ParkingBinding::Reserved(r)) if r.target() == target => reserved += 1,
            _ => (),
        }
    }
    if occupied == 0 {
        return Err(invalid("selected facility has no existing virtual binding"));
    }
    Ok(json!({"occupied":occupied,"reserved":reserved}))
}

fn retained_state(harness: &Harness<'_>) -> String {
    // Used only within a single cutover, whose contract preserves these handles.
    let vehicles: Vec<_> = harness
        .world
        .live_vehicles()
        .iter()
        .map(|h| {
            (
                *h,
                harness.world.vehicle(*h),
                harness.world.parking_binding(*h),
            )
        })
        .collect();
    let routes: Vec<_> = harness
        .world
        .live_routes()
        .map(|h| (h, harness.world.route_edges(h).map(|v| v.to_vec())))
        .collect();
    crate::sha256(format!("{vehicles:?}/{routes:?}").as_bytes())
}

fn extract(harness: &mut Harness<'_>) -> Result<LaneFlowCommittedPoseBatch> {
    let mut batch = LaneFlowCommittedPoseBatch::new();
    checked(
        "cutover pose capture",
        harness
            .adapter_world()?
            .resource_mut::<LaneFlowSession>()
            .extract_committed_pose_batch(FramePlacementToken::new(1), &mut batch),
    )?;
    Ok(batch)
}

fn adapter_cutover(
    harness: &mut Harness<'_>,
    base_reloaded: &Artifacts,
    variant: &CapacityVariant,
) -> Result<Value> {
    let before = retained_state(harness);
    let initial_counts = selected_counts(harness, variant)?;
    let tick = harness.world.tick_index();
    let generation = harness.world.world_generation().get();
    let cursor = harness.world.event_cursor();
    let old_pose = extract(harness)?;
    let same_source = harness.world.committed_source().clone();
    let started = Instant::now();
    let same_events = {
        let mut session = harness.adapter_world()?.resource_mut::<LaneFlowSession>();
        let record = checked(
            "same revision root rebind",
            session.same_revision_restore(
                base_reloaded.revision().clone(),
                same_source,
                LaneFlowTargetSpatial::Rebind(spatial(base_reloaded.revision())?),
                &PREFLIGHT,
            ),
        )?;
        if session.consumption_context_is_current(old_pose.context())
            || record.events().as_slice().len() != 1
        {
            return Err(invalid(
                "same revision rebind did not invalidate context/deliver event",
            ));
        }
        format!("{:?}", record.events())
    };
    let same_ns = started.elapsed().as_nanos();
    if retained_state(harness) != before
        || harness.world.world_generation().get() != generation + 1
        || harness.world.event_cursor() != cursor + 1
        || harness.world.tick_index() != tick
        || !Arc::ptr_eq(&harness.world.revision(), base_reloaded.revision())
    {
        return Err(invalid("same revision rebind changed committed state"));
    }
    let current_pose = extract(harness)?;
    let checkpoint = harness.checkpoint()?;
    {
        let wrong = spatial(base_reloaded.revision())?;
        let mut session = harness.adapter_world()?.resource_mut::<LaneFlowSession>();
        let error = session
            .cross_revision_cutover(
                variant.root.clone(),
                target_source(&variant.root)?,
                &variant.diff,
                variant.binding,
                LaneFlowTargetSpatial::Rebind(wrong),
                &PREFLIGHT,
                &TRANSACTION,
            )
            .err();
        if error != Some(LaneFlowAdapterError::TargetSpatialRevisionMismatch)
            || !session.consumption_context_is_current(current_pose.context())
            || session.world().migration_journal_stats().is_some()
        {
            return Err(invalid("wrong target Spatial did not fail closed"));
        }
    }
    if harness.checkpoint()? != checkpoint {
        return Err(invalid("failed target Spatial mutated state"));
    }
    let started = Instant::now();
    let events = {
        let mut session = harness.adapter_world()?.resource_mut::<LaneFlowSession>();
        let record = checked(
            "Adapter maintenance cutover",
            session.cross_revision_cutover(
                variant.root.clone(),
                target_source(&variant.root)?,
                &variant.diff,
                variant.binding,
                LaneFlowTargetSpatial::Rebind(spatial(&variant.root)?),
                &PREFLIGHT,
                &TRANSACTION,
            ),
        )?;
        if session.consumption_context_is_current(current_pose.context())
            || record.events().as_slice().len() != 1
        {
            return Err(invalid("cross revision cutover context/event mismatch"));
        }
        format!("{:?}", record.events())
    };
    let cross_ns = started.elapsed().as_nanos();
    if retained_state(harness) != before
        || selected_counts(harness, variant)? != initial_counts
        || harness.world.tick_index() != tick
        || harness.world.event_cursor() != cursor + 2
        || harness.world.world_generation().get() != generation + 2
        || !Arc::ptr_eq(&harness.world.revision(), &variant.root)
    {
        return Err(invalid("Adapter capacity migration invariant failed"));
    }
    let fresh = extract(harness)?;
    if fresh.vehicles() != current_pose.vehicles()
        || fresh.batch().records() != current_pose.batch().records()
    {
        return Err(invalid("capacity-only cutover changed committed poses"));
    }
    Ok(
        json!({"same_revision_events":same_events,"cross_revision_events":events,"same_revision_ns":same_ns,
        "cross_revision_ns":cross_ns,"selected_binding_counts":initial_counts,"wrong_spatial":"failed-closed",
        "old_contexts":"rejected","committed_tick":tick}),
    )
}

fn trial(
    artifacts: &Artifacts,
    reloaded: &Artifacts,
    plan: &ResolvedPlan,
    variant: &CapacityVariant,
    mode: &str,
    output: &Path,
) -> Result<Value> {
    fs::create_dir(output)?;
    let mut harness = Harness::install(artifacts, plan)?;
    let adapter = mode != "online";
    if adapter {
        harness = harness.into_adapter(spatial(artifacts.revision())?)?;
    }
    let initial_bindings = selected_counts(&harness, variant)?;
    let mut presentation = Presentation::default();
    let mut log = BufWriter::new(fs::File::create(output.join("ticks.jsonl"))?);
    let mut hashes = Vec::new();
    let mut transition = Value::Null;
    let mut transaction = None;
    let mut save_digest = String::new();
    let mut command_count = 0;
    for tick in 0..END_TICK {
        if tick == SAVE_TICK {
            save_digest = harness.checkpoint()?;
            if mode == "restore" {
                harness = restore(harness, output)?;
                presentation = Presentation::default();
            }
        }
        // Arm before the existing boundary-zero commands, so the online window
        // contains committed lifecycle input as well as real steps.
        if tick == 0 && mode == "online" {
            let world = native(&mut harness)?;
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
                LfcaOriginBinding::from_canonical_origin(*variant.root.canonical_origin()),
                Some(variant.binding),
                MigrationPolicyKind::CrossRevisionDirect,
                world.world_binding(),
            );
            let started = Instant::now();
            transaction = Some(checked(
                "online prepare",
                world.prepare_cross_revision_cutover(
                    variant.root.clone(),
                    target_source(&variant.root)?,
                    &descriptor,
                    &variant.diff,
                    &PREFLIGHT,
                    &TRANSACTION,
                ),
            )?);
            transition = json!({"prepare_tick":tick,"prepare_ns":started.elapsed().as_nanos(),
                    "descriptor":format!("{descriptor:?}"),"prepare_command_cursor":world.command_cursor()});
        }
        if tick == COMMIT_TICK && mode == "online" {
            let before = retained_state(&harness);
            let bindings = selected_counts(&harness, variant)?;
            let generation = harness.world.world_generation().get();
            let cursor = harness.world.event_cursor();
            transition["journal_before_commit"] =
                json!(format!("{:?}", harness.world.migration_journal_stats()));
            let started = Instant::now();
            let commit = checked(
                "online commit",
                transaction
                    .take()
                    .ok_or_else(|| invalid("missing transaction"))?
                    .commit(native(&mut harness)?),
            )?;
            transition["commit_ns"] = json!(started.elapsed().as_nanos());
            transition["events"] = json!(format!("{:?}", commit.events));
            transition["commit_tick"] = json!(tick);
            transition["final_command_cursor"] = json!(commit.final_command_cursor);
            transition["selected_binding_counts"] = bindings.clone();
            if retained_state(&harness) != before
                || selected_counts(&harness, variant)? != bindings
                || commit.events.as_slice().len() != 1
                || harness.world.event_cursor() != cursor + 1
                || harness.world.world_generation().get() != generation + 1
                || harness.world.tick_index() != tick
                || harness.world.migration_journal_stats().is_some()
                || !Arc::ptr_eq(&harness.world.revision(), &variant.root)
                || commit.final_command_cursor
                    <= transition["prepare_command_cursor"]
                        .as_u64()
                        .unwrap_or(u64::MAX)
            {
                return Err(invalid(format!(
                    "online migration invariant: state={} bindings={} events={} cursor={}->{} generation={}->{} tick={} journal={} root={} commands={}->{}",
                    retained_state(&harness) == before,
                    selected_counts(&harness, variant)? == bindings,
                    commit.events.as_slice().len(),
                    cursor,
                    harness.world.event_cursor(),
                    generation,
                    harness.world.world_generation().get(),
                    harness.world.tick_index(),
                    harness.world.migration_journal_stats().is_some(),
                    Arc::ptr_eq(&harness.world.revision(), &variant.root),
                    transition["prepare_command_cursor"],
                    commit.final_command_cursor
                )));
            }
        } else if tick == COMMIT_TICK && mode == "pause" {
            transition = adapter_cutover(&mut harness, reloaded, variant)?;
        }
        let record = harness.advance()?;
        command_count += harness.commands.len();
        let sample = if adapter {
            Some(presentation.sample(&mut harness)?)
        } else {
            None
        };
        let normalized = serde_json::to_vec(&record)?;
        hashes.push(crate::sha256(&normalized));
        serde_json::to_writer(
            &mut log,
            &json!({"runtime":record,"presentation":sample,
            "commands":harness.commands,"events":harness.events}),
        )?;
        log.write_all(b"\n")?;
        if let Some(transaction) = transaction.as_mut() {
            checked("online pump", transaction.pump(native(&mut harness)?))?;
        }
    }
    log.flush()?;
    let result = json!({"mode":mode,"ticks":END_TICK,"save_tick":SAVE_TICK,"commit_tick":COMMIT_TICK,
        "initial_selected_bindings":initial_bindings,"final_selected_bindings":selected_counts(&harness, variant)?,
        "save_digest":save_digest,"final_digest":harness.checkpoint()?,"tick_hashes":hashes,"transition":transition,
        "command_records":command_count,"tile_evidence":harness.evidence,"retry_reasons":harness.error_counts,
        "ticks_file":digest_file(&output.join("ticks.jsonl"))?});
    write_json(&output.join("trial.json"), &result)?;
    Ok(result)
}

fn witness(
    artifacts: &Artifacts,
    reloaded: &Artifacts,
    variant: &CapacityVariant,
    case: UrbanCase,
    output: &Path,
) -> Result<Value> {
    let plan = ResolvedPlan::for_case(artifacts, case, Window::probe(END_TICK)?)?;
    plan.write(&output.join("resolved-plan.toml"))?;
    let baseline = trial(
        artifacts,
        reloaded,
        &plan,
        variant,
        "baseline",
        &output.join("baseline"),
    )?;
    let mut reports = vec![baseline.clone()];
    for mode in ["restore", "online", "pause"] {
        let a = trial(
            artifacts,
            reloaded,
            &plan,
            variant,
            mode,
            &output.join(format!("{mode}-a")),
        )?;
        let b = trial(
            artifacts,
            reloaded,
            &plan,
            variant,
            mode,
            &output.join(format!("{mode}-b")),
        )?;
        for field in [
            "save_digest",
            "final_digest",
            "tick_hashes",
            "tile_evidence",
            "retry_reasons",
        ] {
            if a[field] != b[field] {
                return Err(invalid(format!("{mode} repeat mismatch: {field}")));
            }
            if mode == "restore" && a[field] != baseline[field] {
                return Err(invalid(format!(
                    "save/replay differs from uninterrupted run: {field}"
                )));
            }
        }
        reports.push(a);
        reports.push(b);
    }
    Ok(
        json!({"status":"transition-witness-pass","case":case.as_str(),"scale":artifacts.catalog().scale,
        "save_tick":SAVE_TICK,"online_window":[0,COMMIT_TICK],"end_tick":END_TICK,"trials":reports,
        "transaction_limits":{"journal_bytes":TRANSACTION.max_journal_bytes,"lag_ticks":TRANSACTION.max_catch_up_lag_ticks,"records_per_pump":TRANSACTION.max_records_per_pump},
        "scope":"finite existing demand prefix; replay compares uninterrupted committed results; repeated migrations compare within the same path; broader protocol and failure combinations belong to #538 and are not certified by this witness"}),
    )
}

pub fn run_transition_evidence(
    artifact_directory: &Path,
    variant_directory: &Path,
    case: UrbanCase,
    output: &Path,
) -> Result<Value> {
    if !matches!(case, UrbanCase::MixedPeak | UrbanCase::GarageEgress) {
        return Err(invalid(
            "transition case must be MIXED-PEAK or GARAGE-EGRESS",
        ));
    }
    let started = Instant::now();
    let provenance = crate::evidence::source()?;
    let artifacts = Artifacts::load_spatial(artifact_directory)?;
    let reloaded = Artifacts::load_spatial(artifact_directory)?;
    let variant = load_variant(&artifacts, variant_directory)?;
    let variant_manifest = digest_file(&variant_directory.join("variant.json"))?;
    fs::create_dir(output)?;
    let result = witness(&artifacts, &reloaded, &variant, case, output);
    let mut report = match result {
        Ok(r) => r,
        Err(e) => json!({"status":"failed","error":e.to_string()}),
    };
    if crate::evidence::source()? != provenance
        || digest_file(&variant_directory.join("variant.json"))? != variant_manifest
        || load_variant(&artifacts, variant_directory)?
            .root
            .network_revision()
            != variant.root.network_revision()
        || Artifacts::load_spatial(artifact_directory)?.files != artifacts.files
    {
        return Err(invalid("transition inputs/source changed"));
    }
    report["source"] = provenance;
    report["variant_manifest"] = serde_json::to_value(variant_manifest)?;
    report["artifact_files"] = serde_json::to_value(&artifacts.files)?;
    report["elapsed_ms"] = json!(started.elapsed().as_millis());
    write_json(&output.join("transitions.json"), &report)?;
    if let Some(error) = report["error"].as_str() {
        return Err(invalid(error));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_urban_generator::{Scale, UrbanConfig, generate};

    #[test]
    fn snapshot_replay_and_capacity_cutover_share_the_urban_demand() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let variant_dir = temp.path().join("variant");
        let config =
            UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
        generate(&config, Scale::Fixture, &base, None).unwrap();
        crate::prepare_capacity_variant(&base, &variant_dir).unwrap();
        let artifacts = Artifacts::load_spatial(&base).unwrap();
        let reloaded = Artifacts::load_spatial(&base).unwrap();
        let variant = load_variant(&artifacts, &variant_dir).unwrap();
        for case in [UrbanCase::MixedPeak, UrbanCase::GarageEgress] {
            let out = temp.path().join(case.as_str());
            fs::create_dir(&out).unwrap();
            witness(&artifacts, &reloaded, &variant, case, &out).unwrap();
        }
    }
}
