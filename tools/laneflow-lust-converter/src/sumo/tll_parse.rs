//! Parse `tll.static.xml` additional file into static tlLogic programs.

use roxmltree::{Document, Node};

use crate::{
    Error, Result,
    sumo::{net::SumoTlLogic, net_parse::parse_tl_logic},
};

/// Parse a SUMO additional file containing static `<tlLogic>` programs.
pub fn parse_tll_static_xml(xml: &str) -> Result<Vec<SumoTlLogic>> {
    let document = Document::parse(xml).map_err(|source| Error::XmlParse(source.to_string()))?;
    let root = document.root_element();
    let mut logics = Vec::new();
    collect_tl_logics(root, &mut logics)?;
    if logics.is_empty() {
        return Err(Error::SumoModel(
            "tll.static.xml contains no <tlLogic> elements".to_owned(),
        ));
    }
    logics.sort_by(|left, right| left.id.cmp(&right.id));
    for window in logics.windows(2) {
        if window[0].id == window[1].id {
            return Err(Error::SumoModel(format!(
                "duplicate tlLogic id {:?} in tll.static.xml",
                window[0].id
            )));
        }
    }
    Ok(logics)
}

fn collect_tl_logics(node: Node<'_, '_>, logics: &mut Vec<SumoTlLogic>) -> Result<()> {
    if node.is_element() && node.tag_name().name() == "tlLogic" {
        logics.push(parse_tl_logic(node)?);
        return Ok(());
    }
    for child in node.children().filter(Node::is_element) {
        collect_tl_logics(child, logics)?;
    }
    Ok(())
}
