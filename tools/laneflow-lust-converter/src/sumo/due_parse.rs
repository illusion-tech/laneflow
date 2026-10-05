//! Parse DUE `local.static.*.rou.xml` vehicle/route records.

use std::str::FromStr;

use roxmltree::{Document, Node};

use crate::{
    Error, Result,
    sumo::{decimal::ExactDecimal, due::DueVehicle},
};

/// Parse one DUE routes file, materializing `<route>` edges only for records
/// matching `keep_edges(type_id, depart)`（population 候选谓词）。
///
/// 全网三份 DUE 共 215,526 辆、8,771,017 条 edge 引用，而入选谓词只依赖
/// type/depart——非候选的 edges 永不被消费（入选集必为候选、候选必物化），
/// 跳过物化可省约 95% 的 edge String 分配。非候选记录以空 edge 列表占位
/// （与缺 route 形态一致；入选集失败域不受影响）。结构形态失败的解析期
/// 口径与物化与否无关：多个内联 route、空 edges 仍对全部记录拒绝。
pub fn parse_due_routes_xml_filtered(
    xml: &str,
    source_file_ordinal: u8,
    keep_edges: impl Fn(&str, &ExactDecimal) -> bool,
) -> Result<Vec<DueVehicle>> {
    if source_file_ordinal > 2 {
        return Err(Error::SumoModel(format!(
            "DUE source_file_ordinal must be 0..=2, got {source_file_ordinal}"
        )));
    }
    let document = Document::parse(xml).map_err(|source| Error::XmlParse(source.to_string()))?;
    let mut vehicles = Vec::new();
    let mut ordinal = 0_u64;
    collect_vehicles(
        document.root_element(),
        source_file_ordinal,
        &mut ordinal,
        &mut vehicles,
        &keep_edges,
    )?;
    Ok(vehicles)
}

fn collect_vehicles(
    node: Node<'_, '_>,
    source_file_ordinal: u8,
    ordinal: &mut u64,
    vehicles: &mut Vec<DueVehicle>,
    keep_edges: &impl Fn(&str, &ExactDecimal) -> bool,
) -> Result<()> {
    if node.is_element() && node.tag_name().name() == "vehicle" {
        vehicles.push(parse_vehicle(
            node,
            source_file_ordinal,
            *ordinal,
            keep_edges,
        )?);
        *ordinal = ordinal
            .checked_add(1)
            .ok_or_else(|| Error::SumoModel("DUE vehicle ordinal overflowed u64".to_owned()))?;
        return Ok(());
    }
    for child in node.children().filter(Node::is_element) {
        collect_vehicles(child, source_file_ordinal, ordinal, vehicles, keep_edges)?;
    }
    Ok(())
}

fn parse_vehicle(
    node: Node<'_, '_>,
    source_file_ordinal: u8,
    source_vehicle_ordinal: u64,
    keep_edges: &impl Fn(&str, &ExactDecimal) -> bool,
) -> Result<DueVehicle> {
    // #253 U3：空 id 失败域限在入选集——解析期保留原始 id（空也收），
    // selection 在选取集合内 fail-closed。
    let id = required_attr(node, "id")?;
    let type_id = required_attr(node, "type")?;
    let depart = ExactDecimal::from_str(&required_attr(node, "depart")?)?;
    let depart_pos = node.attribute("departPos").map(str::to_owned);
    // #253 Q9：恰好一个内联 <route>——取首个会静默忽略其余声明。多个 route
    // 是结构形态问题，解析期即拒（失败域口径不变）；缺失 route 按 K3(a) 契约
    // 失败域口径推迟到入选集（select_population），解析期原样保留（缺失也收）。
    let mut routes = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "route");
    let Some(route) = routes.next() else {
        return Ok(DueVehicle {
            id,
            type_id,
            depart,
            road_edge_ids: Vec::new(),
            depart_pos,
            source_file_ordinal,
            source_vehicle_ordinal,
        });
    };
    if routes.next().is_some() {
        return Err(Error::SumoModel(format!(
            "DUE vehicle {id:?} declares multiple inline <route> elements"
        )));
    }
    let edges_raw = required_attr(route, "edges")?;
    // 空 edges 是结构形态问题，对全部记录解析期拒绝（与物化与否无关）。
    if edges_raw.split_whitespace().next().is_none() {
        return Err(Error::SumoModel(format!(
            "DUE vehicle {id:?} route has no edges"
        )));
    }
    let road_edge_ids = if keep_edges(&type_id, &depart) {
        edges_raw
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Ok(DueVehicle {
        id,
        type_id,
        depart,
        road_edge_ids,
        depart_pos,
        source_file_ordinal,
        source_vehicle_ordinal,
    })
}

fn required_attr(node: Node<'_, '_>, name: &str) -> Result<String> {
    node.attribute(name).map(str::to_owned).ok_or_else(|| {
        Error::SumoModel(format!(
            "<{}> missing required attribute @{name}",
            node.tag_name().name()
        ))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn multiple_inline_routes_fail_closed() {
        // #253 Q9：一个 vehicle 多个内联 <route> 不再静默取首个。
        let xml = r#"<routes>
  <vehicle id="v0" type="passenger1" depart="28800">
    <route edges="a b"/>
    <route edges="c d"/>
  </vehicle>
</routes>"#;
        let error = super::parse_due_routes_xml_filtered(xml, 0, |_, _| true)
            .expect_err("multiple inline routes must fail");
        assert!(
            error.to_string().contains("multiple inline <route>"),
            "{error}"
        );
    }

    #[test]
    fn filtered_parse_skips_edges_for_non_candidates_only() {
        // 非候选不物化 edges（空 Vec 占位）；候选照常物化。结构失败域不变：
        // 非候选的空 edges / 多 route 仍在解析期拒绝。
        let xml = r#"<routes>
  <vehicle id="keep" type="passenger1" depart="28800">
    <route edges="a b"/>
  </vehicle>
  <vehicle id="skip" type="bus" depart="28800">
    <route edges="c d e"/>
  </vehicle>
</routes>"#;
        let vehicles =
            super::parse_due_routes_xml_filtered(xml, 0, |type_id, _| type_id == "passenger1")
                .expect("filtered parse");
        assert_eq!(vehicles[0].road_edge_ids, ["a", "b"]);
        assert!(
            vehicles[1].road_edge_ids.is_empty(),
            "non-candidate edges not materialized"
        );

        let empty_edges = r#"<routes>
  <vehicle id="x" type="bus" depart="28800">
    <route edges=""/>
  </vehicle>
</routes>"#;
        let error =
            super::parse_due_routes_xml_filtered(empty_edges, 0, |_, _| false).expect_err("empty");
        assert!(error.to_string().contains("no edges"), "{error}");

        let multi = r#"<routes>
  <vehicle id="y" type="bus" depart="28800">
    <route edges="a"/>
    <route edges="b"/>
  </vehicle>
</routes>"#;
        let error =
            super::parse_due_routes_xml_filtered(multi, 0, |_, _| false).expect_err("multi");
        assert!(
            error.to_string().contains("multiple inline <route>"),
            "{error}"
        );
    }

    #[test]
    fn missing_inline_route_keeps_record_at_parse_time() {
        // #253 W1：缺失 route 按 K3(a) 失败域口径推迟到入选集——解析期原样
        // 保留（缺失也收），不整文件失败；present 但 edges 为空的结构形态问题
        // 仍在解析期拒绝（口径不变）。
        let xml = r#"<routes>
  <vehicle id="v0" type="passenger1" depart="28800">
    <route edges="a b"/>
  </vehicle>
  <vehicle id="v1" type="bus" depart="28800"/>
</routes>"#;
        let vehicles = super::parse_due_routes_xml_filtered(xml, 0, |_, _| true)
            .expect("missing route tolerated");
        assert_eq!(vehicles.len(), 2);
        assert_eq!(vehicles[0].road_edge_ids, ["a", "b"]);
        assert!(
            vehicles[1].road_edge_ids.is_empty(),
            "missing route kept as empty edge list"
        );

        let empty_edges = r#"<routes>
  <vehicle id="v2" type="passenger1" depart="28800">
    <route edges=""/>
  </vehicle>
</routes>"#;
        let error = super::parse_due_routes_xml_filtered(empty_edges, 0, |_, _| true)
            .expect_err("empty edges fail");
        assert!(error.to_string().contains("no edges"), "{error}");
    }
}
