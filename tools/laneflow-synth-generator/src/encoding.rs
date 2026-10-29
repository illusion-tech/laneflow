use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, Identity};

pub(crate) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    output
}

pub(crate) fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    fn write(value: &Value, out: &mut Vec<u8>) -> Result<(), Error> {
        match value {
            Value::Object(map) => {
                out.push(b'{');
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_unstable_by_key(|(left, _)| *left);
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index != 0 {
                        out.push(b',');
                    }
                    serde_json::to_writer(&mut *out, key)?;
                    out.push(b':');
                    write(value, out)?;
                }
                out.push(b'}');
            }
            Value::Array(values) => {
                out.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        out.push(b',');
                    }
                    write(value, out)?;
                }
                out.push(b']');
            }
            Value::Number(number) if !number.is_i64() && !number.is_u64() => {
                return Err(Error::Invalid("规范 JSON 禁止浮点数".into()));
            }
            _ => serde_json::to_writer(out, value)?,
        }
        Ok(())
    }
    let mut out = Vec::new();
    write(&serde_json::to_value(value)?, &mut out)?;
    out.push(b'\n');
    Ok(out)
}

pub(crate) fn config_bytes(identity: &Identity) -> Result<Vec<u8>, Error> {
    canonical_json(&serde_json::json!({"format":"lf-synth-config-1", "identity":identity}))
}
