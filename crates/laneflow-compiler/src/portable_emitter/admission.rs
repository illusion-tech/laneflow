//! 消费端在分配索引前借用扫描 LFCA；不接收来源处理限额。

use laneflow_static_contract::CanonicalNetworkCapacity;

use super::{CanonicalInputSide, PortableEmissionError, RegistryCheckedObjectView};

pub(super) fn admit_canonical_view(
    view: RegistryCheckedObjectView<'_>,
    capacity: &CanonicalNetworkCapacity,
    side: CanonicalInputSide,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    let usage = super::canonical_usage::lfca_statistics(view, mismatch)?.usage();
    capacity
        .admit(&usage)
        .map_err(|violation| PortableEmissionError::CanonicalCapacityExceeded { side, violation })
}
