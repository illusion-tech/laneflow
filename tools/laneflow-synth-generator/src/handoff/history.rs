use serde_json::Value;

use crate::Identity;

/// 核对 L1 的四个检查点、各点可见的历史前缀，以及各簇两名竞争者的完整交接。
///
/// # Errors
/// 拍号、槽位、路线出现顺序或车尾清空与场景不一致时返回错误。
pub fn check_history(checkpoints: &[Value], identity: &Identity) -> Result<(), String> {
    identity.validate().map_err(|e| e.to_string())?;
    let params = identity.lane_change().ok_or("缺少换道簇参数")?;
    let (warm, observation) = identity.lane_change_window();
    let end = warm + observation;
    let expected_ticks = [0, warm, 2 * warm, end];
    if checkpoints.len() != expected_ticks.len() {
        return Err("L1 检查点数量不符".into());
    }
    let rows = checkpoints[3]["handoff_journal"]
        .as_array()
        .ok_or("最终检查点缺少横移历史")?;
    let mut phases = vec![0_u32; params.clusters as usize];
    let mut last_tick = vec![0; params.clusters as usize];
    let mut last_row = None;
    let first_route = identity.cells() * 4;
    for row in rows {
        let cells = row
            .as_array()
            .filter(|cells| cells.len() == 4)
            .ok_or("横移记录须有四列")?;
        let tick = cells[0].as_u64().ok_or("横移记录缺少拍号")?;
        let slot = cells[1]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("缺少有效换道槽位")?;
        let cluster = (slot / 2) as usize;
        let phase = phases.get_mut(cluster).ok_or("换道槽位超出簇数")?;
        if *phase >= 6
            || slot % 2 != *phase / 3
            || cells[2] != 10 + *phase % 3
            || cells[3] != first_route + slot
            || tick <= last_tick[cluster]
            || tick >= end
            || (*phase == 0 && tick != 1)
            || last_row.is_some_and(|previous| (tick, slot) <= previous)
        {
            return Err(format!(
                "横移竞争顺序、路线或阶段不符：拍 {tick}，槽位 {slot}"
            ));
        }
        *phase += 1;
        last_tick[cluster] = tick;
        last_row = Some((tick, slot));
    }
    if phases.iter().any(|&phase| phase != 6) {
        return Err("每簇两车均须完成进入、前杠交接和车尾清空".into());
    }
    for (point, tick) in checkpoints.iter().zip(expected_ticks) {
        let end = rows.partition_point(|row| row[0].as_u64().is_some_and(|at| at <= tick));
        if point["tick"] != tick
            || point["handoff_journal"].as_array().map(Vec::as_slice) != Some(&rows[..end])
        {
            return Err(format!("横移历史与检查点不符：拍 {tick}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Case, LaneChangeParams};
    use serde_json::json;

    #[test]
    fn separate_clusters_can_advance_together_but_require_their_own_pair() {
        let identity = Identity::new(
            10_000,
            Case::LaneChange(LaneChangeParams {
                clusters: 2,
                changers_per_cluster: 2,
            }),
            16,
        )
        .unwrap();
        let (warm, observation) = identity.lane_change_window();
        let mut history = Vec::new();
        for (i, tick) in [1, 30, 45, 46, 50, 60].into_iter().enumerate() {
            for cluster in 0..2 {
                let slot = cluster * 2 + i / 3;
                history.push(json!([tick, slot, 10 + i % 3, 400 + slot]));
            }
        }
        let mut checkpoints = [0, warm, 2 * warm, warm + observation].map(|tick| {
            let n = history.partition_point(|row| row[0].as_u64().unwrap() <= tick);
            json!({"tick":tick,"handoff_journal":&history[..n]})
        });
        check_history(&checkpoints, &identity).unwrap();
        checkpoints[3]["handoff_journal"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(check_history(&checkpoints, &identity).is_err());
    }
}
