//! Fixture 自验：geometry 编译输出与同语义 Synthetic 孪生逐表核对（切片 6 等价模式），
//! 以及 MIN workload 的九组合不变性。孪生不能表达 facility band 几何行，两处已知差异
//! （`facility_band_geometries` 与 `canonical_points`）按精确公式单独断言。

use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler, GeometryAccuracyProfile,
    GeometryDirectionProfile,
};
use laneflow_static_contract::FieldTag;

use crate::corridor::CorridorModel;
use crate::counts::{GeometrySource, WorkloadCounts, compile_geometry_workload};
use crate::twin::{build_corridor_twin, harvest_geometry_output};

/// 基准等价组合：孪生点输入来自该组合的派生输出。
pub const EQUIVALENCE_PROFILES: (GeometryAccuracyProfile, GeometryDirectionProfile) = (
    GeometryAccuracyProfile::Balanced5Cm,
    GeometryDirectionProfile::Balanced2Deg,
);

/// corridor/P100 等价自验：编译 geometry workload、收获派生值、构造并编译孪生，
/// 然后逐表核对 53 张 LIR 表（两处已知差异按公式断言），并逐 edge 比点位。
pub fn check_corridor_equivalence(
    model: &CorridorModel,
    geometry_modules: &[GeometrySource<'_>],
    twin_namespaces: &[String],
    copies: u64,
) {
    let (accuracy, direction) = EQUIVALENCE_PROFILES;
    let (geometry_output, _) = compile_geometry_workload(geometry_modules, accuracy, direction);
    let harvest = harvest_geometry_output(&geometry_output);
    let limits = CompileLimits::p100_initial_v1();
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for (index, namespace) in twin_namespaces.iter().enumerate() {
        let document_key = geometry_modules[index].document_key;
        // P100 各副本命名空间不同、几何内容相同，消费同一份收获。
        let twin = build_corridor_twin(model, namespace, document_key, &limits, &harvest);
        unit.add_synthetic_module(twin)
            .expect("孪生模块进入编译单元");
    }
    let twin_output = Compiler::new()
        .compile(unit.build().expect("孪生编译单元"))
        .expect("孪生 workload 必须可编译");

    let geometry_tables = geometry_output.lir().lir_table_counts();
    let twin_tables = twin_output.lir().lir_table_counts();
    let band_count = u64::try_from(
        model
            .roads
            .iter()
            .map(|road| road.bands.len())
            .sum::<usize>(),
    )
    .unwrap();
    for (name, geometry_count) in geometry_tables.entries() {
        let twin_count = twin_tables.get(name).unwrap();
        match name {
            "facility_band_geometries" => {
                assert_eq!(
                    geometry_count,
                    band_count * copies,
                    "facility_band_geometries 应等于副本数 × 声明 band 数"
                );
                assert_eq!(twin_count, 0, "孪生不能表达 facility band 几何行");
            }
            "canonical_points" => {
                // 收获取自与 geometry 相同的副本数，band 点数已跨副本合计。
                assert_eq!(
                    geometry_count.saturating_sub(twin_count),
                    harvest.band_point_count,
                    "canonical_points 差值应等于全部副本的 band 点总数"
                );
            }
            _ => assert_eq!(
                geometry_count, twin_count,
                "LIR 表 {name} 在 geometry 与孪生之间不一致"
            ),
        }
    }

    // 逐 edge 比点位：孪生消费的点输入必须等于它自己输出的点（harvest 传递正确性）。
    let twin_lir = twin_output.lir();
    for edge in twin_lir.lane_edges() {
        let key = edge
            .identity_fields()
            .find(|field| field.tag() == FieldTag::LaneEdgeKey)
            .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
            .expect("lane edge 必须携带 LaneEdgeKey");
        let expected = &harvest.edges[&key].points;
        let actual: Vec<[f32; 3]> = edge
            .spatial_geometry()
            .expect("孪生 edge 必须携带空间几何")
            .points()
            .map(|point| [point.x, point.y, point.z])
            .collect();
        assert_eq!(&actual, expected, "edge {key} 的孪生点位与收获不一致");
    }
}

/// MIN 九组合不变性：同一 line-only 文档在全部位置/方向组合下的 LIR 表、语义指纹与
/// 规范点数必须逐位相同（§9.2：只改变选项摘要、不制造多余点）。
pub fn check_min_invariance(document_key: &str, source: &[u8]) {
    let namespace = "calibration/geometry/min";
    let mut baseline: Option<(laneflow_compiler::LirTableCounts, [u8; 32], u64)> = None;
    for accuracy in crate::counts::ACCURACY_PROFILES {
        for direction in crate::counts::DIRECTION_PROFILES {
            let modules = [GeometrySource {
                namespace,
                document_key,
                source,
            }];
            let (_, counts): (CompilationOutput, WorkloadCounts) =
                compile_geometry_workload(&modules, accuracy, direction);
            let tables = counts.lir_table_counts.expect("编译成功即填充 LIR 表计数");
            let snapshot = (
                tables,
                counts.semantic_fingerprint,
                counts.canonical_point_count,
            );
            match &baseline {
                None => baseline = Some(snapshot),
                Some(expected) => assert_eq!(
                    &snapshot, expected,
                    "MIN 在 ({accuracy:?}, {direction:?}) 组合下与基准组合不一致"
                ),
            }
        }
    }
}
