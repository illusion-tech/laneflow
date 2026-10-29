//! #536 的有界安全输入 harness；仅供 fuzz 与种子验证，不属于产品 API。
#![forbid(unsafe_code)]

mod cutover;
mod portable;
mod snapshot;

use sha2::{Digest, Sha256};

pub use cutover::check_cutover;
pub use portable::{check_portable_bytes, check_portable_mutation};
pub use snapshot::check_snapshot;

/// 本次 fuzz 的单输入上限；生产上限仍由各生产入口拥有。
pub const MAX_INPUT_BYTES: usize = 262_144;

/// 初始语料及其目标与文件名；内容同时用于本地回归和 runner 语料初始化。
///
/// # Panics
///
/// 已登记的合法夹具不能通过生产入口时触发 oracle，不能静默降为空语料。
pub fn corpus_seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let mut seeds = portable::seeds();
    seeds.extend(snapshot::seeds());
    seeds.extend(cutover::seeds());
    seeds.push(("lfre_writer_differential", "identity", vec![0; 4]));
    seeds.push((
        "lfre_arbitrary_bytes",
        "writer",
        snapshot::lfre_module("roads/fuzz"),
    ));
    seeds
}

/// 语料库存的互操作字节摘要；只用于证据文件，不进入生产或稳态 tick。
pub fn corpus_digest(bytes: &[u8]) -> String {
    format!(
        "{:x}",
        laneflow_static_contract::Sha256Digest::from_bytes(Sha256::digest(bytes).into())
    )
}

/// 变异程序至多执行 32 次，绝不按输入声明值分配。空程序保持合法种子。
fn mutate(seed: &[u8], program: &[u8]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    for op in program.as_chunks::<5>().0.iter().take(32) {
        let offset = usize::from(u16::from_le_bytes([op[1], op[2]]));
        match op[0] % 5 {
            0 => bytes.truncate(offset % (bytes.len() + 1)),
            1 | 2 if !bytes.is_empty() => {
                let index = offset % bytes.len();
                if op[0] % 5 == 1 {
                    bytes[index] ^= op[3];
                } else {
                    bytes[index] = op[3];
                }
            }
            3 if !bytes.is_empty() => {
                let index = offset % bytes.len();
                let count = usize::from(op[4]).min(bytes.len() - index);
                bytes[index..index + count].fill(op[3]);
            }
            4 if bytes.len() < MAX_INPUT_BYTES => bytes.push(op[3]),
            _ => {}
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_seed_reaches_its_registered_entry() {
        for (target, _, bytes) in corpus_seeds() {
            match target {
                "portable_arbitrary_bytes" => check_portable_bytes(&bytes),
                "portable_bundle_mutation" => check_portable_mutation(&bytes),
                "lfrs_restore" => check_snapshot(&bytes),
                "cutover_inputs" => check_cutover(&bytes),
                "lfre_arbitrary_bytes" | "lfre_writer_differential" => {}
                _ => panic!("unregistered target"),
            }
        }
    }
}
