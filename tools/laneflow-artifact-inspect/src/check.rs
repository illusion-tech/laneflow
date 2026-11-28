//! 单对象完整预检：[`inspect`](crate::inspect) 与 [`diff`](crate::diff) 共用的唯一检查入口。

use laneflow_format::{
    FormatError, FormatLimits, RegistryCheckedObjectView, ValueCheckedObjectView,
    preflight_object_values, without_digest_probe,
};
use laneflow_static_contract::PortableObjectKind;

/// 在无摘要探针的闭包内依次完成 registry 结构预检与直接值域检查。
///
/// 只有两层都通过的对象才返回 registry 视图供展示与比较；任一层失败均返回
/// 对应 [`FormatError`]，损坏输入不写 #794 取证转储。
pub(crate) fn preflight_object(
    bytes: &[u8],
    kind: PortableObjectKind,
    format_limits: FormatLimits,
) -> Result<RegistryCheckedObjectView<'_>, FormatError> {
    without_digest_probe(|| {
        preflight_object_values(bytes, kind, format_limits)
            .map(ValueCheckedObjectView::registry_view)
    })
}
