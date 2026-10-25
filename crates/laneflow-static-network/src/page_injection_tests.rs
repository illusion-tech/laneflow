//! #963 实际整数 backing 的页级注入；只在唯一拥有、没有活动借用时写入。
//! 地址证明虚拟页范围，不证明物理页帧、物理共页或自然硬件故障。

mod fingerprint_tests;

use std::{
    fs,
    hint::black_box,
    path::Path,
    process::{Child, Command},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, LaneEdgeInput, PortableDiffBase,
    PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use serde::{Deserialize, Serialize};

use crate::{
    AttestationOutcome, AttestationVerdict, Disposition, MismatchSite, RootAttestationExpectation,
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    attest_shared_network_revision, build_shared_network_revision,
};

const PAGE: usize = 4_096;
const WORDS: usize = 4_096;
const NEAR_ZERO_MAX_NONZERO: usize = 16;
const NEAR_ZERO_MIN_CLEARED: usize = PAGE / 16;
const PROTOCOL: &str = "laneflow-page-injection-v1";
const CHILD_TEST: &str = "page_injection_tests::child_entry";
type Result<T> = std::result::Result<T, String>;

fn supports_collection(system_page: usize) -> bool {
    cfg!(any(unix, windows)) && system_page == PAGE
}

fn skip_unsupported_page_regression() -> bool {
    let system_page = page_size::get();
    if supports_collection(system_page) {
        false
    } else {
        eprintln!(
            "SKIP page regression: unsupported platform or page size {system_page}; no page coverage"
        );
        true
    }
}

fn ensure(condition: bool, message: &str) -> Result<()> {
    condition.then_some(()).ok_or_else(|| message.to_owned())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum Fault {
    Healthy,
    Zero,
    Replaced,
    ShiftBefore,
    ShiftAfter,
    Partial,
}

const FAULTS: [Fault; 6] = [
    Fault::Healthy,
    Fault::Zero,
    Fault::Replaced,
    Fault::ShiftBefore,
    Fault::ShiftAfter,
    Fault::Partial,
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum Behavior {
    Normal,
    Exit,
    Wait,
}

#[derive(Deserialize, Serialize)]
struct Request {
    fault: Fault,
    control: bool,
    behavior: Behavior,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Geometry {
    allocation_address: usize,
    allocation_bytes: usize,
    page_bytes: usize,
    page_address: usize,
    word_offset: usize,
}

impl Geometry {
    fn at(address: usize, bytes: usize, system_page: usize) -> Result<Self> {
        ensure(
            system_page == PAGE,
            "unsupported system page size; no page coverage",
        )?;
        ensure(
            address.is_multiple_of(4) && bytes.is_multiple_of(4),
            "unaligned u32 column",
        )?;
        let first = address.checked_add(PAGE - 1).ok_or("address overflow")? / PAGE * PAGE;
        let page_address = first.checked_add(PAGE).ok_or("page address overflow")?;
        let end = address
            .checked_add(bytes)
            .ok_or("allocation end overflow")?;
        ensure(
            page_address
                .checked_add(2 * PAGE)
                .is_some_and(|value| value <= end),
            "column must contain target and both complete donor pages",
        )?;
        Ok(Self {
            allocation_address: address,
            allocation_bytes: bytes,
            page_bytes: system_page,
            page_address,
            word_offset: (page_address - address) / 4,
        })
    }

    fn range(&self) -> std::ops::Range<usize> {
        self.word_offset..self.word_offset + PAGE / 4
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct Witness {
    verdict: String,
    cause: Option<String>,
    disposition: String,
    mismatch: Option<TableEvidence>,
    source_audit: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TableEvidence {
    stage: String,
    component: String,
    table: String,
    entity_kind: Option<String>,
    index: u32,
    field: String,
    expected: String,
    actual: String,
}

#[derive(Debug, Serialize)]
struct Verification {
    fingerprint: String,
    target_offset_bytes: usize,
    first_difference_byte_offset: Option<usize>,
    different_bytes: usize,
}

#[derive(Deserialize, Serialize)]
struct Response {
    protocol: String,
    pid: u32,
    geometry: Geometry,
    after_address: usize,
    after_call_address: usize,
    native_endian: String,
    baseline: Option<Witness>,
    attestation: Option<Witness>,
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn snapshot(words: &[u32]) -> Vec<u8> {
    black_box(words)
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

fn fixture() -> Result<Vec<u8>> {
    let limits = CompileLimits::single_network_1m_v2();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "test/page-injection",
            source_document_key: "page-injection.document",
            generator_build_id: "page-injection-v1",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: None,
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .map_err(|e| format!("source header: {e:?}"))?;
    let mut module =
        SyntheticModuleBuilder::new(header, &limits).map_err(|e| format!("module: {e:?}"))?;
    for index in 0..WORDS {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: &format!("edge-{index:05}"),
                length_meters: (1_000 + index) as f64,
                speed_limit_meters_per_second: 13.0,
                successors: &[],
            })
            .map_err(|e| format!("lane: {e:?}"))?;
    }
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(
        module
            .finish()
            .map_err(|e| format!("finish module: {e:?}"))?,
    )
    .map_err(|e| format!("add module: {e:?}"))?;
    let output = Compiler::new()
        .compile(unit.build().map_err(|e| format!("unit: {e:?}"))?)
        .map_err(|e| format!("compile: {e:?}"))?;
    let provenance = PortableEmissionProvenance::try_new("page-injection-v1")
        .map_err(|e| format!("provenance: {e:?}"))?;
    Ok(emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .map_err(|e| format!("emit: {e:?}"))?
    .canonical_artifact()
    .bytes()
    .to_vec())
}

fn options() -> SharedNetworkBuildOptions<'static> {
    SharedNetworkBuildOptions::new(
        SpatialBuildOption::Omit,
        SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
    )
}

fn build(bytes: &[u8]) -> Result<(Arc<SharedNetworkRevision>, RootAttestationExpectation)> {
    let checked =
        check_canonical_network_input(bytes, FormatLimits::HARD).map_err(|e| format!("{e:?}"))?;
    // 引用来自仍健康的来源 reader，绝不从待改写修订提取期望。
    let expected = RootAttestationExpectation::new(
        checked.network_revision(),
        checked.canonical_artifact_digest(),
        checked.canonical_artifact_byte_length(),
    );
    let root = build_shared_network_revision(checked, options()).map_err(|e| format!("{e:?}"))?;
    Ok((root, expected))
}

fn attest(
    root: &SharedNetworkRevision,
    bytes: &[u8],
    expected: RootAttestationExpectation,
) -> Result<Witness> {
    let report = match attest_shared_network_revision(
        root,
        bytes,
        expected,
        FormatLimits::HARD,
        options(),
        None,
    ) {
        AttestationOutcome::Concluded(report) => report,
        failure => {
            return Err(format!(
                "authentication did not produce a verdict: {failure:?}"
            ));
        }
    };
    let mismatch = match report.mismatch {
        None => None,
        Some(evidence) => match evidence.site {
            MismatchSite::TableValue {
                component,
                table,
                entity_kind,
                index,
                field,
                expected,
                actual,
                ..
            } => Some(TableEvidence {
                stage: format!("{:?}", evidence.stage),
                component: format!("{component:?}"),
                table: table.to_owned(),
                entity_kind: entity_kind.map(|kind| format!("{kind:?}")),
                index,
                field: field.to_owned(),
                expected,
                actual,
            }),
            other => return Err(format!("unexpected evidence: {other:?}")),
        },
    };
    Ok(Witness {
        verdict: format!("{:?}", report.verdict),
        cause: report.cause.map(|v| format!("{v:?}")),
        disposition: format!("{:?}", report.disposition),
        mismatch,
        source_audit: report
            .source_audit
            .iter()
            .map(|v| format!("{v:?}"))
            .collect(),
    })
}

fn pass(witness: &Witness) -> Result<()> {
    ensure(
        witness.verdict == format!("{:?}", AttestationVerdict::Pass)
            && witness.cause.is_none()
            && witness.disposition == format!("{:?}", Disposition::None)
            && witness.mismatch.is_none()
            && witness.source_audit.is_empty(),
        "healthy control was not a clean Pass",
    )
}

fn mutate(words: &mut [u32], geometry: &Geometry, fault: Fault) {
    let range = geometry.range();
    match fault {
        Fault::Healthy => (),
        Fault::Zero => words[range].fill(0),
        Fault::Replaced => words[range].fill(0xA5A5_5A5A),
        Fault::ShiftBefore => words.copy_within(range.start - PAGE / 4..range.start, range.start),
        Fault::ShiftAfter => words.copy_within(range.end..range.end + PAGE / 4, range.start),
        Fault::Partial => words[range.start] ^= 1,
    }
    black_box(words);
}

fn control_words() -> Vec<u32> {
    (0..WORDS as u32)
        .map(|index| index.wrapping_mul(73_129) ^ 0x29A7_45B1)
        .collect()
}

fn child(directory: &Path, source: &Path) -> Result<()> {
    let request: Request = read_json(&directory.join("request.json"))?;
    match request.behavior {
        Behavior::Exit => std::process::exit(67),
        Behavior::Wait => {
            thread::sleep(Duration::from_secs(3));
            return Err("deliberate delayed child".into());
        }
        Behavior::Normal => (),
    }
    let bytes = if request.control {
        Vec::new()
    } else {
        fs::read(source).map_err(|e| e.to_string())?
    };
    let mut root = None;
    let mut expected = None;
    let mut baseline = None;
    let mut control = Vec::new();
    let words: &mut [u32] = if request.control {
        control = control_words();
        &mut control
    } else {
        let (value, reference) = build(&bytes)?;
        baseline = Some(attest(&value, &bytes, reference)?);
        pass(baseline.as_ref().unwrap())?;
        expected = Some(reference);
        root = Some(value);
        &mut Arc::get_mut(root.as_mut().unwrap())
            .ok_or("root must be uniquely owned")?
            .traffic
            .lane_lengths_millimetres
    };
    ensure(
        cfg!(any(unix, windows)),
        "system page query is not supported on this platform",
    )?;
    let geometry = Geometry::at(
        words.as_ptr().addr(),
        std::mem::size_of_val(words),
        page_size::get(),
    )?;
    fs::write(directory.join("before.bin"), snapshot(words)).map_err(|e| e.to_string())?;
    mutate(words, &geometry, request.fault);
    let after_address = words.as_ptr().addr();
    ensure(
        words.as_ptr().addr() == geometry.allocation_address,
        "mutation moved the allocation",
    )?;
    fs::write(directory.join("after.bin"), snapshot(words)).map_err(|e| e.to_string())?;
    // 可变借用已在此结束；之后只读真实修订，不使用阶段视图钩子。
    let attestation = root
        .as_ref()
        .map(|root| attest(root, &bytes, expected.unwrap()))
        .transpose()?;
    let final_words = root.as_ref().map_or(control.as_slice(), |root| {
        root.traffic.lane_lengths_millimetres()
    });
    let after_call_address = final_words.as_ptr().addr();
    fs::write(directory.join("after-call.bin"), snapshot(final_words))
        .map_err(|e| e.to_string())?;
    write_json(
        &directory.join("response.json"),
        &Response {
            protocol: PROTOCOL.to_owned(),
            pid: std::process::id(),
            geometry,
            after_address,
            after_call_address,
            native_endian: if cfg!(target_endian = "little") {
                "little"
            } else {
                "big"
            }
            .to_owned(),
            baseline,
            attestation,
        },
    )
}

#[test]
#[ignore = "independent child entry; invoked by page evidence parent"]
fn child_entry() {
    let directory = std::env::var_os("LF_PAGE_CASE").expect("LF_PAGE_CASE");
    let source = std::env::var_os("LF_PAGE_SOURCE").expect("LF_PAGE_SOURCE");
    child(Path::new(&directory), Path::new(&source)).unwrap();
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn collect_child(directory: &Path, source: &Path, timeout: Duration) -> Result<u32> {
    let started = Instant::now();
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args(["--ignored", "--exact", CHILD_TEST, "--nocapture"])
            .env("LF_PAGE_CASE", directory)
            .env("LF_PAGE_SOURCE", source)
            .stdout(fs::File::create(directory.join("stdout.txt")).map_err(|e| e.to_string())?)
            .stderr(fs::File::create(directory.join("stderr.txt")).map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let pid = child.0.id();
    write_json(
        &directory.join("child.json"),
        &serde_json::json!({
        "pid":pid, "parent_pid":std::process::id(), "timeout_ms":timeout.as_millis()}),
    )?;
    loop {
        ensure(started.elapsed() < timeout, "child timed out")?;
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            ensure(status.success(), &format!("child failed: {status}"))?;
            return Ok(pid);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

// 根据读回内容判读，不按要求的 Fault 预填指纹。范围均为完整、同分配内的页。
fn fingerprint(before: &[u8], after: &[u8], offset: usize) -> String {
    let page = &after[offset..offset + PAGE];
    if page == &before[offset..offset + PAGE] {
        return "Unchanged".into();
    }
    if page.iter().all(|byte| *byte == 0) {
        return "Zero".into();
    }
    for delta in [-1_isize, 1] {
        let donor = offset.checked_add_signed(delta * PAGE as isize).unwrap();
        if page == &before[donor..donor + PAGE] {
            return format!("Shifted({delta})");
        }
    }
    // 对齐 validation-matrix §8.1；保留精确供体优先，局部清零不得提升为写丢失页。
    let nonzero = page.iter().filter(|byte| **byte != 0).count();
    let cleared = before[offset..offset + PAGE]
        .iter()
        .zip(page)
        .filter(|(left, right)| **left != 0 && **right == 0)
        .count();
    if nonzero <= NEAR_ZERO_MAX_NONZERO && cleared >= NEAR_ZERO_MIN_CLEARED {
        return "Zero".into();
    }
    let changed_segments = (0..16)
        .filter(|segment| {
            let start = offset + segment * (PAGE / 16);
            before[start..start + PAGE / 16] != after[start..start + PAGE / 16]
        })
        .count();
    if changed_segments >= 12 {
        "Replaced"
    } else {
        "Partial"
    }
    .into()
}

fn verify(directory: &Path, request: &Request, pid: u32, golden: &[u8]) -> Result<Verification> {
    let response: Response = read_json(&directory.join("response.json"))?;
    ensure(
        response.protocol == PROTOCOL && response.pid == pid && pid != std::process::id(),
        "response provenance mismatch",
    )?;
    ensure(
        response.native_endian
            == if cfg!(target_endian = "little") {
                "little"
            } else {
                "big"
            },
        "native endian mismatch",
    )?;
    let geometry = Geometry::at(
        response.geometry.allocation_address,
        golden.len(),
        response.geometry.page_bytes,
    )?;
    ensure(
        geometry.page_address == response.geometry.page_address
            && geometry.word_offset == response.geometry.word_offset
            && geometry.allocation_bytes == response.geometry.allocation_bytes
            && response.after_address == geometry.allocation_address
            && response.after_call_address == geometry.allocation_address,
        "unproved page range",
    )?;
    let before = fs::read(directory.join("before.bin")).map_err(|e| e.to_string())?;
    let after = fs::read(directory.join("after.bin")).map_err(|e| e.to_string())?;
    let after_call = fs::read(directory.join("after-call.bin")).map_err(|e| e.to_string())?;
    ensure(
        before == golden && after.len() == golden.len() && after_call == after,
        "missing or changed readback",
    )?;
    let offset = geometry.word_offset * 4;
    let mut expected = before.clone();
    match request.fault {
        Fault::Healthy => (),
        Fault::Zero => expected[offset..offset + PAGE].fill(0),
        Fault::Replaced => {
            for word in expected[offset..offset + PAGE].as_chunks_mut::<4>().0 {
                word.copy_from_slice(&0xA5A5_5A5A_u32.to_ne_bytes());
            }
        }
        Fault::ShiftBefore => {
            expected[offset..offset + PAGE].copy_from_slice(&before[offset - PAGE..offset])
        }
        Fault::ShiftAfter => expected[offset..offset + PAGE]
            .copy_from_slice(&before[offset + PAGE..offset + 2 * PAGE]),
        Fault::Partial => {
            let word = u32::from_ne_bytes(before[offset..offset + 4].try_into().unwrap()) ^ 1;
            expected[offset..offset + 4].copy_from_slice(&word.to_ne_bytes());
        }
    }
    ensure(
        after == expected,
        "actual readback differs from specified mutation or changed outside target",
    )?;
    let healthy = matches!(request.fault, Fault::Healthy);
    ensure(
        healthy || before != after,
        "injection was a no-op; no coverage",
    )?;
    if request.control {
        ensure(
            response.baseline.is_none() && response.attestation.is_none(),
            "control group consumed LaneFlow authentication",
        )?;
    } else {
        pass(response.baseline.as_ref().ok_or("missing baseline")?)?;
        let result = response
            .attestation
            .as_ref()
            .ok_or("missing authentication")?;
        if healthy {
            pass(result)?;
        } else {
            ensure(
                result.verdict == "Mismatch"
                    && result.cause.as_deref() == Some("SharedStaticRoot")
                    && result.disposition == "DisableRoot"
                    && result.source_audit.is_empty(),
                "unexpected verdict or attribution",
            )?;
            let mismatch = result
                .mismatch
                .as_ref()
                .ok_or("missing mismatch evidence")?;
            let expected_value = u32::from_ne_bytes(before[offset..offset + 4].try_into().unwrap());
            let actual_value = u32::from_ne_bytes(after[offset..offset + 4].try_into().unwrap());
            ensure(
                mismatch.stage == "RootComparison"
                    && mismatch.component == "Traffic"
                    && mismatch.table == "lane_lengths_millimetres"
                    && mismatch.entity_kind.is_none()
                    && mismatch.index as usize == geometry.word_offset
                    && mismatch.field == "value"
                    && mismatch.expected == expected_value.to_string()
                    && mismatch.actual == actual_value.to_string(),
                "mismatch does not identify actual changed memory",
            )?;
        }
    }
    let classification = fingerprint(&before, &after, offset);
    let expected_class = match request.fault {
        Fault::Healthy => "Unchanged",
        Fault::Zero => "Zero",
        Fault::Replaced => "Replaced",
        Fault::ShiftBefore => "Shifted(-1)",
        Fault::ShiftAfter => "Shifted(1)",
        Fault::Partial => "Partial",
    };
    ensure(
        classification == expected_class,
        "page fingerprint differs from intended pattern",
    )?;
    Ok(Verification {
        fingerprint: classification,
        target_offset_bytes: offset,
        first_difference_byte_offset: before.iter().zip(&after).position(|(a, b)| a != b),
        different_bytes: before.iter().zip(&after).filter(|(a, b)| a != b).count(),
    })
}

fn run_case(
    directory: &Path,
    source: &Path,
    request: &Request,
    golden: &[u8],
    timeout: Duration,
) -> Result<()> {
    fs::create_dir(directory).map_err(|e| e.to_string())?;
    write_json(
        &directory.join("result.json"),
        &serde_json::json!({"status":"Incomplete", "reason":"audit pending"}),
    )?;
    let result = (|| {
        write_json(&directory.join("request.json"), request)?;
        let pid = collect_child(directory, source, timeout)?;
        verify(directory, request, pid, golden)
    })();
    match result {
        Ok(verification) => write_json(
            &directory.join("result.json"),
            &serde_json::json!({"status":"Complete", "verification":verification}),
        ),
        Err(error) => {
            write_json(
                &directory.join("result.json"),
                &serde_json::json!({"status":"Incomplete", "reason":error}),
            )?;
            Err(error)
        }
    }
}

fn matrix(directory: &Path) -> Result<()> {
    matrix_for_page_size(directory, page_size::get())
}

fn matrix_for_page_size(directory: &Path, system_page: usize) -> Result<()> {
    // 拒绝已有目录时不写任何失败记录；只有原子创建成功后才拥有此目录。
    fs::create_dir(directory).map_err(|e| e.to_string())?;
    let result = (|| {
        write_json(
            &directory.join("run.json"),
            &serde_json::json!({"status":"Incomplete"}),
        )?;
        ensure(
            supports_collection(system_page),
            "unsupported system page size or platform; no page coverage",
        )?;
        let bytes = fixture()?;
        let source = directory.join("network.lfca");
        fs::write(&source, &bytes).map_err(|e| e.to_string())?;
        let checked = check_canonical_network_input(bytes.as_slice(), FormatLimits::HARD)
            .map_err(|e| format!("source registration: {e:?}"))?;
        write_json(
            &directory.join("source.json"),
            &serde_json::json!({
                "network_revision":checked.network_revision().as_digest().as_bytes(),
                "canonical_artifact_digest":checked.canonical_artifact_digest().as_bytes(),
                "exact_bytes":bytes.len(), "reference_from_source_not_root":true
            }),
        )?;
        let (root, expected) = build(&bytes)?;
        pass(&attest(&root, &bytes, expected)?)?;
        let root_golden = snapshot(root.traffic.lane_lengths_millimetres());
        let control_golden = snapshot(&control_words());
        for control in [false, true] {
            for fault in FAULTS {
                let case = directory.join(format!(
                    "{}-{fault:?}",
                    if control { "control" } else { "root" }
                ));
                run_case(
                    &case,
                    &source,
                    &Request {
                        fault,
                        control,
                        behavior: Behavior::Normal,
                    },
                    if control {
                        &control_golden
                    } else {
                        &root_golden
                    },
                    Duration::from_secs(30),
                )?;
            }
        }
        ensure(
            fs::read(&source).map_err(|e| e.to_string())? == bytes,
            "source artifact changed",
        )?;
        Ok(())
    })();
    match &result {
        Ok(()) => write_json(
            &directory.join("run.json"),
            &serde_json::json!({
            "protocol":PROTOCOL, "status":"Complete", "parent_pid":std::process::id(),
            "root_faults":5, "root_healthy_controls":1, "environment_fault_controls":5,
            "environment_healthy_controls":1, "page_bytes":system_page,
            "os":std::env::consts::OS, "arch":std::env::consts::ARCH,
            "source_commit":std::env::var("LF_PAGE_COMMIT").ok(),
            "physical_page_frame_or_common_page_proved":false }),
        )?,
        Err(error) => write_json(
            &directory.join("run.json"),
            &serde_json::json!({"status":"Incomplete", "reason":error}),
        )?,
    }
    result
}

#[test]
fn actual_pages_are_injected_and_verified_in_independent_children() {
    if skip_unsupported_page_regression() {
        return;
    }
    let temporary = tempfile::tempdir().unwrap();
    let result = matrix(&temporary.path().join("evidence"));
    if let Err(error) = result {
        panic!("{error}; retained evidence: {}", temporary.keep().display());
    }
}

#[test]
fn geometry_rejects_unsupported_or_incomplete_pages() {
    assert!(Geometry::at(0x10_0000, PAGE * 4, PAGE).is_ok());
    assert!(Geometry::at(0x10_0000, PAGE * 4, PAGE * 2).is_err());
    assert!(Geometry::at(0x10_0001, PAGE * 4, PAGE).is_err());
    assert!(Geometry::at(0x10_0004, PAGE * 2, PAGE).is_err());
    assert!(Geometry::at(usize::MAX - 3, PAGE * 4, PAGE).is_err());
}

#[test]
fn existing_evidence_is_never_overwritten() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("run.json"), b"sentinel").unwrap();
    assert!(matrix(temporary.path()).is_err());
    assert_eq!(
        fs::read(temporary.path().join("run.json")).unwrap(),
        b"sentinel"
    );
}

#[test]
fn failed_and_timed_out_children_remain_incomplete() {
    let temporary = tempfile::tempdir().unwrap();
    for (index, behavior) in [Behavior::Exit, Behavior::Wait].into_iter().enumerate() {
        let directory = temporary.path().join(format!("failed-{index}"));
        let request = Request {
            fault: Fault::Zero,
            control: true,
            behavior,
        };
        let timeout = if matches!(behavior, Behavior::Exit) {
            Duration::from_secs(30)
        } else {
            Duration::from_millis(100)
        };
        assert!(
            run_case(
                &directory,
                &temporary.path().join("unused"),
                &request,
                &snapshot(&control_words()),
                timeout
            )
            .is_err()
        );
        let result: serde_json::Value = read_json(&directory.join("result.json")).unwrap();
        assert_eq!(result["status"], "Incomplete");
        assert!(result["reason"].as_str().unwrap().contains(
            if matches!(behavior, Behavior::Exit) {
                "child failed"
            } else {
                "timed out"
            }
        ));
        assert!(!directory.join("response.json").exists());
    }
}

#[test]
fn missing_short_foreign_and_noop_evidence_is_rejected() {
    if skip_unsupported_page_regression() {
        return;
    }
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("control");
    let request = Request {
        fault: Fault::Healthy,
        control: true,
        behavior: Behavior::Normal,
    };
    let golden = snapshot(&control_words());
    run_case(
        &directory,
        &temporary.path().join("no-lfca-for-control"),
        &request,
        &golden,
        Duration::from_secs(30),
    )
    .unwrap();
    let mut response: Response = read_json(&directory.join("response.json")).unwrap();
    let pid = response.pid;
    fs::write(directory.join("after.bin"), &golden[..golden.len() - 1]).unwrap();
    assert!(verify(&directory, &request, pid, &golden).is_err());
    fs::write(directory.join("after.bin"), &golden).unwrap();
    assert!(verify(&directory, &request, std::process::id(), &golden).is_err());
    fs::remove_file(directory.join("after-call.bin")).unwrap();
    assert!(verify(&directory, &request, pid, &golden).is_err());
    let zeros = vec![0; golden.len()];
    for name in ["before.bin", "after.bin", "after-call.bin"] {
        fs::write(directory.join(name), &zeros).unwrap();
    }
    let noop = Request {
        fault: Fault::Zero,
        control: true,
        behavior: Behavior::Normal,
    };
    assert!(
        verify(&directory, &noop, pid, &zeros)
            .unwrap_err()
            .contains("no-op")
    );
    for name in ["before.bin", "after.bin", "after-call.bin"] {
        fs::write(directory.join(name), &golden).unwrap();
    }
    response.after_call_address += 4;
    write_json(&directory.join("response.json"), &response).unwrap();
    assert!(
        verify(&directory, &request, pid, &golden)
            .unwrap_err()
            .contains("unproved page range")
    );
    response.after_call_address -= 4;
    response.geometry.word_offset += 1;
    write_json(&directory.join("response.json"), &response).unwrap();
    assert!(verify(&directory, &request, pid, &golden).is_err());
}

#[test]
#[ignore = "bounded manual collection; LF_PAGE_OUTPUT must name a new directory"]
fn collect_page_evidence() {
    let output = std::env::var_os("LF_PAGE_OUTPUT").expect("LF_PAGE_OUTPUT");
    matrix(Path::new(&output)).unwrap();
}

#[test]
fn unsupported_collection_is_incomplete_before_source_or_children() {
    let temporary = tempfile::tempdir().unwrap();
    for system_page in [PAGE * 2, PAGE * 4, PAGE * 16] {
        let directory = temporary.path().join(format!("unsupported-{system_page}"));
        assert!(!supports_collection(system_page));
        assert!(
            matrix_for_page_size(&directory, system_page)
                .unwrap_err()
                .contains("unsupported")
        );
        let result: serde_json::Value = read_json(&directory.join("run.json")).unwrap();
        assert_eq!(result["status"], "Incomplete");
        assert!(result["reason"].as_str().unwrap().contains("unsupported"));
        assert!(!directory.join("network.lfca").exists());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    }
}
