//! current JSON Core loader 已拆除；本模块不再构造可运行交通世界。

use crate::corridor::CorridorTemplate;

pub(crate) fn build_production_loader_template() -> Result<CorridorTemplate, String> {
    Err("current JSON Core loader was removed in #301".to_owned())
}

pub(crate) fn build_production_loader_fixture_case(
    _case_id: &str,
) -> Result<CorridorTemplate, String> {
    Err("current JSON Core loader was removed in #301".to_owned())
}
