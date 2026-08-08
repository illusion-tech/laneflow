//! 生成 MIN / CORRIDOR / P100 三个校准 workload fixture 容器：两轮发射（roads-only
//! 探针编译收获派生 lane 端点，再生成真实连接曲线）、九组合编译自验、孪生逐表等价核对，
//! 最后写出容器文件并打印字节身份。

use std::collections::BTreeMap;
use std::path::PathBuf;

use issue_296_geometry_frontend_calibration::container::{
    FixtureModule, encode_container, sha256_hex,
};
use issue_296_geometry_frontend_calibration::corridor::CorridorModel;
use issue_296_geometry_frontend_calibration::counts::{GeometrySource, compile_geometry_workload};
use issue_296_geometry_frontend_calibration::emit::{
    connect_curves, emit_corridor_document, emit_min_document, emit_probe_document,
};
use issue_296_geometry_frontend_calibration::selfcheck::{
    check_corridor_equivalence, check_min_invariance,
};
use issue_296_geometry_frontend_calibration::twin::harvest_geometry_output;

const MIN_WORKLOAD_ID: &str = "LF-COMP-GEOMETRY-MIN-v1";
const CORRIDOR_WORKLOAD_ID: &str = "LF-COMP-GEOMETRY-CORRIDOR-v1";
const P100_WORKLOAD_ID: &str = "LF-COMP-GEOMETRY-P100-v1";

const MIN_NAMESPACE: &str = "calibration/geometry/min";
const CORRIDOR_NAMESPACE: &str = "calibration/geometry/corridor";
const MIN_DOCUMENT_KEY: &str = "min.geometry.json";
const CORRIDOR_DOCUMENT_KEY: &str = "corridor.geometry.json";

const TRAFFIC_FIXTURE: &str =
    include_str!("../../../../examples/data/v0.10-signalized-corridor.laneflow.json");

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn main() {
    let traffic: serde_json::Value =
        serde_json::from_str(TRAFFIC_FIXTURE).expect("traffic fixture 必须是合法 JSON");
    let model = CorridorModel::parse(&traffic);
    eprintln!(
        "走廊模型：{} roads / {} junctions / {} internal edges / {} routes / {} gates",
        model.roads.len(),
        model.junctions.len(),
        model.internal_edges.len(),
        model.routes.len(),
        model.gates.len()
    );

    // 第一轮：roads-only 探针编译，收获派生 lane 端点（lane 派生不依赖 junction/overlay，
    // 探针端点与完整文档逐位一致）。
    let probe_document = emit_probe_document(
        &model,
        CORRIDOR_NAMESPACE,
        CORRIDOR_DOCUMENT_KEY,
        "#296 geometry calibration corridor (probe pass)",
    );
    let probe_modules = [GeometrySource {
        namespace: CORRIDOR_NAMESPACE,
        document_key: CORRIDOR_DOCUMENT_KEY,
        source: probe_document.as_bytes(),
    }];
    let (probe_output, _) = compile_geometry_workload(
        &probe_modules,
        laneflow_compiler::GeometryAccuracyProfile::Balanced5Cm,
        laneflow_compiler::GeometryDirectionProfile::Balanced2Deg,
    );
    let probe_harvest = harvest_geometry_output(&probe_output);
    let endpoints: BTreeMap<String, ([f32; 3], [f32; 3])> = probe_harvest
        .edges
        .iter()
        .map(|(key, edge)| {
            let first = *edge.points.first().expect("lane edge 至少一个点");
            let last = *edge.points.last().expect("lane edge 至少一个点");
            (key.clone(), (first, last))
        })
        .collect();

    // 第二轮：真实连接曲线。
    let curves = connect_curves(&model, &endpoints);
    let corridor_document = emit_corridor_document(
        &model,
        CORRIDOR_NAMESPACE,
        CORRIDOR_DOCUMENT_KEY,
        "#296 geometry calibration corridor v1",
        &curves,
    );

    // CORRIDOR：九组合编译 + 孪生逐表等价。
    let corridor_modules = [GeometrySource {
        namespace: CORRIDOR_NAMESPACE,
        document_key: CORRIDOR_DOCUMENT_KEY,
        source: corridor_document.as_bytes(),
    }];
    for accuracy in issue_296_geometry_frontend_calibration::counts::ACCURACY_PROFILES {
        for direction in issue_296_geometry_frontend_calibration::counts::DIRECTION_PROFILES {
            compile_geometry_workload(&corridor_modules, accuracy, direction);
        }
    }
    eprintln!("CORRIDOR 九组合编译通过");
    check_corridor_equivalence(
        &model,
        &corridor_modules,
        &[CORRIDOR_NAMESPACE.to_string()],
        1,
    );
    eprintln!("CORRIDOR 孪生逐表等价通过");

    // P100：五份独立命名空间副本（各自独立 document key，编译单元内键唯一），
    // 九组合编译 + 孪生逐表等价。
    let p100_namespaces: Vec<String> = (0..5)
        .map(|index| format!("calibration/geometry/p100/{index:02}"))
        .collect();
    let p100_document_keys: Vec<String> = (0..5)
        .map(|index| format!("corridor-{index:02}.geometry.json"))
        .collect();
    let p100_documents: Vec<String> = p100_namespaces
        .iter()
        .zip(p100_document_keys.iter())
        .map(|(namespace, document_key)| {
            emit_corridor_document(
                &model,
                namespace,
                document_key,
                "#296 geometry calibration p100 corridor copy",
                &curves,
            )
        })
        .collect();
    let p100_modules: Vec<GeometrySource<'_>> = p100_namespaces
        .iter()
        .zip(p100_document_keys.iter())
        .zip(p100_documents.iter())
        .map(|((namespace, document_key), document)| GeometrySource {
            namespace,
            document_key,
            source: document.as_bytes(),
        })
        .collect();
    for accuracy in issue_296_geometry_frontend_calibration::counts::ACCURACY_PROFILES {
        for direction in issue_296_geometry_frontend_calibration::counts::DIRECTION_PROFILES {
            compile_geometry_workload(&p100_modules, accuracy, direction);
        }
    }
    eprintln!("P100 九组合编译通过");
    check_corridor_equivalence(&model, &p100_modules, &p100_namespaces, 5);
    eprintln!("P100 孪生逐表等价通过");

    // MIN：九组合不变性。
    let min_document = emit_min_document(MIN_NAMESPACE, MIN_DOCUMENT_KEY);
    check_min_invariance(MIN_DOCUMENT_KEY, min_document.as_bytes());
    eprintln!("MIN 九组合不变性通过");

    // 写容器文件并打印字节身份。
    let fixtures = fixtures_dir();
    std::fs::create_dir_all(&fixtures).expect("创建 fixtures 目录");
    let containers = [
        (
            MIN_WORKLOAD_ID,
            "min-v1.fixture.json",
            vec![FixtureModule {
                source_path: MIN_DOCUMENT_KEY.to_string(),
                source: min_document,
            }],
        ),
        (
            CORRIDOR_WORKLOAD_ID,
            "corridor-v1.fixture.json",
            vec![FixtureModule {
                source_path: CORRIDOR_DOCUMENT_KEY.to_string(),
                source: corridor_document,
            }],
        ),
        (
            P100_WORKLOAD_ID,
            "p100-v1.fixture.json",
            p100_document_keys
                .iter()
                .zip(p100_documents)
                .map(|(document_key, document)| FixtureModule {
                    source_path: document_key.clone(),
                    source: document,
                })
                .collect(),
        ),
    ];
    for (workload_id, file_name, modules) in containers {
        let bytes = encode_container(workload_id, &modules);
        let path = fixtures.join(file_name);
        std::fs::write(&path, &bytes).expect("写 fixture 容器");
        println!(
            "{workload_id}\t{}\t{}\t{}",
            file_name,
            bytes.len(),
            sha256_hex(&bytes)
        );
    }
    eprintln!("全部 fixture 生成并自验通过");
}
