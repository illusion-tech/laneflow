//! 消费端在分配索引前借用扫描 LFCA；不接收来源处理限额。

use laneflow_static_contract::CanonicalNetworkCapacity;

use super::{CanonicalInputSide, PortableEmissionError, RegistryCheckedObjectView};

pub(super) fn admit_canonical_view(
    view: RegistryCheckedObjectView<'_>,
    capacity: &CanonicalNetworkCapacity,
    side: CanonicalInputSide,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    let section = view.section(8).ok_or(mismatch)?;
    let limit = capacity
        .get(laneflow_static_contract::CanonicalNetworkDimension::GeometryPointCount)
        .min(16_000_000);
    for (ordinal, table) in [
        (1, "StationAnchor"),
        (4, "StationSpan"),
        (6, "BoundaryExtent"),
    ] {
        let required = u64::from(section.table(ordinal).ok_or(mismatch)?.row_count());
        if required > limit {
            return Err(PortableEmissionError::StationCapacityExceeded {
                side,
                table,
                required,
                limit,
            });
        }
    }
    let usage = super::canonical_usage::lfca_statistics(view, mismatch)?.usage();
    capacity
        .admit(&usage)
        .map_err(|violation| PortableEmissionError::CanonicalCapacityExceeded { side, violation })
}
