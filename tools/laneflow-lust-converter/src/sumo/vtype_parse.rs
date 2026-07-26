//! Parse `vtypes.add.xml` into SUMO vType records.

use std::str::FromStr;

use roxmltree::{Document, Node};

use crate::{
    Error, Result,
    sumo::{decimal::ExactDecimal, vtype::SumoVType},
};

/// Parse a SUMO routes/additional file containing `<vType>` definitions.
pub fn parse_vtypes_xml(xml: &str) -> Result<Vec<SumoVType>> {
    let document = Document::parse(xml).map_err(|source| Error::XmlParse(source.to_string()))?;
    let mut vtypes = Vec::new();
    collect_vtypes(document.root_element(), &mut vtypes)?;
    if vtypes.is_empty() {
        return Err(Error::SumoModel(
            "vtypes file contains no <vType> elements".to_owned(),
        ));
    }
    vtypes.sort_by(|left, right| left.id.cmp(&right.id));
    for window in vtypes.windows(2) {
        if window[0].id == window[1].id {
            return Err(Error::SumoModel(format!(
                "duplicate vType id {:?}",
                window[0].id
            )));
        }
    }
    Ok(vtypes)
}

fn collect_vtypes(node: Node<'_, '_>, vtypes: &mut Vec<SumoVType>) -> Result<()> {
    if node.is_element() && node.tag_name().name() == "vType" {
        vtypes.push(parse_vtype(node)?);
    }
    for child in node.children().filter(Node::is_element) {
        collect_vtypes(child, vtypes)?;
    }
    Ok(())
}

fn parse_vtype(node: Node<'_, '_>) -> Result<SumoVType> {
    Ok(SumoVType {
        id: required_attr(node, "id")?,
        v_class: required_attr(node, "vClass")?,
        accel: ExactDecimal::from_str(&required_attr(node, "accel")?)?,
        decel: ExactDecimal::from_str(&required_attr(node, "decel")?)?,
        length: ExactDecimal::from_str(&required_attr(node, "length")?)?,
        min_gap: ExactDecimal::from_str(&required_attr(node, "minGap")?)?,
        max_speed: ExactDecimal::from_str(&required_attr(node, "maxSpeed")?)?,
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
