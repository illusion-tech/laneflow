//! `lust.poly.xml` 的 parking polygon 健康事实解析（#253 K6）。
//!
//! §3.5 契约：parking polygons 只作为 source health 与转换报告事实。这里
//! 最小化解析——只数 `type="parking"` 的 `<poly>` 元素，不做几何处理；
//! pinned 基线为 175。

use roxmltree::Document;

use crate::{Error, Result};

/// Count `<poly type="parking">` entries（LuST v1 parking 健康事实）。
pub fn parse_parking_polygon_count(xml: &str) -> Result<u64> {
    let document = Document::parse(xml)
        .map_err(|source| Error::XmlParse(format!("lust.poly.xml is not valid XML: {source}")))?;
    let count = document
        .descendants()
        .filter(|node| node.has_tag_name("poly"))
        .filter(|node| node.attribute("type") == Some("parking"))
        .count();
    u64::try_from(count)
        .map_err(|_| Error::SumoModel("parking polygon count overflows u64".to_owned()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_only_parking_typed_polys() {
        let xml = r#"<?xml version="1.0"?>
<additional>
  <poly id="p1" type="parking" shape="0,0 1,0 1,1"/>
  <poly id="p2" type="parking" shape="0,0 1,0 1,1"/>
  <poly id="b1" type="building" shape="0,0 1,0 1,1"/>
</additional>"#;
        assert_eq!(super::parse_parking_polygon_count(xml).expect("count"), 2);
    }

    #[test]
    fn invalid_poly_xml_fails_closed() {
        assert!(super::parse_parking_polygon_count("<additional>").is_err());
    }
}
