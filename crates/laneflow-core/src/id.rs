//! external ID token 校验。

use crate::error::CoreError;

pub(crate) const EXTERNAL_ID_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$";

const EXTERNAL_ID_MAX_LEN: usize = 128;

pub(crate) fn validate_external_id(
    field: &'static str,
    external_id: &str,
) -> Result<(), CoreError> {
    if is_valid_external_id(external_id) {
        Ok(())
    } else {
        Err(CoreError::InvalidExternalId {
            field,
            external_id: external_id.to_owned(),
            pattern: EXTERNAL_ID_PATTERN,
        })
    }
}

fn is_valid_external_id(external_id: &str) -> bool {
    if external_id.is_empty() || external_id.len() > EXTERNAL_ID_MAX_LEN {
        return false;
    }

    let mut bytes = external_id.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };

    if !first.is_ascii_alphanumeric() {
        return false;
    }

    bytes.all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_id_token_accepts_data_format_pattern() {
        for external_id in ["A", "edge_1", "road/1:lane-2.3"] {
            assert!(is_valid_external_id(external_id));
        }
    }

    #[test]
    fn external_id_token_rejects_invalid_values() {
        let too_long = "a".repeat(129);
        for external_id in ["", "_edge", "-edge", "edge 1", "车道1", too_long.as_str()] {
            assert!(!is_valid_external_id(external_id));
        }
    }
}
