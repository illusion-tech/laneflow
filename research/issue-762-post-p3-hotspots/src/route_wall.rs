//! #679 输出语法的本包解析器；Rust 源码加载保持在本包审计边界内。
use std::collections::BTreeMap;
type Key = (u32, bool, usize);
type Samples = BTreeMap<Key, Vec<u64>>;
fn fields(line: &str) -> BTreeMap<&str, &str> {
    line.split_whitespace()
        .skip(1)
        .filter_map(|item| item.split_once('='))
        .collect()
}

pub(crate) fn wall(text: &str) -> Result<Samples, String> {
    let mut samples: Samples = BTreeMap::new();
    let mut ends = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    for line in text.lines().filter(|line| line.starts_with("route-")) {
        let row = fields(line);
        let value = |name| {
            row.get(name)
                .copied()
                .ok_or_else(|| format!("missing {name}"))
        };
        let number = |name| {
            value(name)?
                .parse::<u64>()
                .map_err(|_| format!("invalid {name}"))
        };
        let round = number("round")?;
        let red = match value("red")? {
            "true" => true,
            "false" => false,
            _ => return Err("red".into()),
        };
        let repeats = number("repeats")?;
        if round >= 3 || ![8, 128, 2_048].contains(&repeats) {
            return Err("matrix axis".into());
        }
        let key = (round as u32, red, repeats as usize);
        if line.starts_with("route-tick ") {
            let values = samples.entry(key).or_default();
            if number("tick")? != values.len() as u64 || values.len() >= 128 || number("ns")? == 0 {
                return Err("sample order/count/time".into());
            }
            values.push(number("ns")?);
        } else if line.starts_with("route-end ") {
            if ends.insert(key, number("build_ns")?).is_some() {
                return Err("duplicate end".into());
            }
            if number("vehicles")? != 512
                || number("warm")? != 16
                || number("steps")? != 128
                || number("state_sum")? != 1_332_736_691
            {
                return Err("workload identity".into());
            }
            let input = value("input")?;
            if inputs
                .insert(red, input)
                .is_some_and(|previous| previous != input)
            {
                return Err("input drift".into());
            }
        } else {
            return Err("unknown row".into());
        }
    }
    if samples.len() != 18
        || ends.len() != 18
        || samples.values().any(|values| values.len() != 128)
        || samples.keys().ne(ends.keys())
        || !text.contains("test result: ok. 1 passed; 0 failed;")
    {
        return Err("incomplete matrix".into());
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::wall;
    const VALID: &str = include_str!("../../issue-679-route-query/evidence/wall.log");

    #[test]
    fn historical_wall_matrix_is_complete() {
        let samples = wall(VALID).unwrap();
        assert_eq!(samples.len(), 18);
        assert!(samples.values().all(|values| values.len() == 128));
    }

    #[test]
    fn rejects_missing_duplicate_and_bad_samples() {
        let row = VALID
            .lines()
            .find(|line| line.starts_with("route-tick "))
            .unwrap();
        assert!(wall(&VALID.replacen(row, "", 1)).is_err());
        assert!(wall(&VALID.replacen(row, &format!("{row}\n{row}"), 1)).is_err());
        assert!(wall(&VALID.replacen("repeats=8", "repeats=9", 1)).is_err());
        assert!(wall(&VALID.replace("state_sum=1332736691", "state_sum=0")).is_err());
    }
}
