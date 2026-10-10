//! #814：在冻结源码副本中标记公共 step，生产源码不增加探针。
use crate::{Result, need};
use std::{fs, path::Path};

const CLOCK: &str = "                let started = Instant::now();\n                let result = world.step(input);\n                let elapsed = nanos(started.elapsed());";

fn patch(text: &str) -> Result<String> {
    need(
        text.matches(CLOCK).count() == 1,
        "public step Windows clock anchor differs",
    )?;
    let replacement = r#"                let begin = lf814_wpr::Anchor::begin()?;
                let started = Instant::now();
                let result = world.step(input);
                let finished = Instant::now();
                let elapsed = nanos(finished.duration_since(started));
                if let Some(begin) = begin {
                    let end = lf814_wpr::Anchor::now()?;
                    lf814_wpr::Window::new(begin, end, started, finished)?.emit(world.tick_index());
                }"#;
    Ok(format!(
        "{}\n#[path = \"columnar_wpr_window.rs\"]\nmod lf814_wpr;\n",
        text.replacen(CLOCK, replacement, 1)
    ))
}

pub(crate) fn instrument(source: &Path) -> Result<()> {
    let host = source.join("tools/laneflow-urban-harness/src/host.rs");
    let original = fs::read_to_string(&host)?.replace("\r\n", "\n");
    fs::write(host, patch(&original)?)?;
    fs::write(
        source.join("tools/laneflow-urban-harness/src/columnar_wpr_window.rs"),
        include_str!("columnar_wpr_window.rs"),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_export_requires_one_public_step_and_emits_after_latency_clock() {
        assert!(patch("").is_err());
        assert!(patch(&format!("{CLOCK}\n{CLOCK}")).is_err());
        let result = patch(CLOCK).unwrap();
        assert_eq!(result.matches("world.step(input)").count(), 1);
        assert!(result.find("let elapsed").unwrap() < result.find("?.emit(").unwrap());
        assert!(result.contains("mod lf814_wpr;"));
    }
}
