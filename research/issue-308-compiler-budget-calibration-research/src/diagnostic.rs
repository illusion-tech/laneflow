//! 冻结研究诊断流的生产者编码。
//!
//! 这里仅供被测研究实现写出诊断摘要；Evidence v1 验证器必须保留自己的独立重建，
//! 不能调用本模块后比较同源结果。

use sha2::{Digest, Sha256};

type DiagnosticCanonicalKey<'a> = (&'a [u8], u32, u32, u32, u32, &'a [u8], u8, &'a [u8]);

pub(crate) const DIAGNOSTIC_STREAM_DOMAIN: &[u8] = b"LANEFLOW-COMPILER-CALIBRATION-DIAGNOSTIC-V1\0";
pub(crate) const DIAGNOSTIC_STREAM_VERSION: u32 = 1;
const ERROR_SEVERITY: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DiagnosticSource {
    pub(crate) document_key: String,
    pub(crate) start_line: u32,
    pub(crate) start_column: u32,
    pub(crate) end_line: u32,
    pub(crate) end_column: u32,
}

impl DiagnosticSource {
    pub(crate) fn absent() -> Self {
        Self {
            document_key: String::new(),
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticRecord {
    pub(crate) code: &'static str,
    pub(crate) source: DiagnosticSource,
    pub(crate) typed_payload: Vec<u8>,
}

impl DiagnosticRecord {
    fn canonical_key(&self) -> DiagnosticCanonicalKey<'_> {
        (
            self.source.document_key.as_bytes(),
            self.source.start_line,
            self.source.start_column,
            self.source.end_line,
            self.source.end_column,
            self.code.as_bytes(),
            ERROR_SEVERITY,
            self.typed_payload.as_slice(),
        )
    }
}

pub(crate) fn empty_diagnostic_digest() -> String {
    diagnostic_digest(Vec::new())
}

pub(crate) fn limit_exceeded_diagnostic_digest(
    error_code: &'static str,
    dimension_code_u8: u8,
    selected_limit_value: u64,
    observed_value: u64,
) -> String {
    let mut typed_payload = Vec::with_capacity(17);
    typed_payload.push(dimension_code_u8);
    typed_payload.extend_from_slice(&selected_limit_value.to_le_bytes());
    typed_payload.extend_from_slice(&observed_value.to_le_bytes());
    diagnostic_digest(vec![DiagnosticRecord {
        code: error_code,
        source: DiagnosticSource::absent(),
        typed_payload,
    }])
}

pub(crate) fn diagnostic_digest(mut diagnostics: Vec<DiagnosticRecord>) -> String {
    diagnostics.sort_unstable_by(|left, right| left.canonical_key().cmp(&right.canonical_key()));
    let mut stream = Vec::new();
    stream.extend_from_slice(DIAGNOSTIC_STREAM_DOMAIN);
    stream.extend_from_slice(&DIAGNOSTIC_STREAM_VERSION.to_le_bytes());
    stream.extend_from_slice(
        &u64::try_from(diagnostics.len())
            .expect("diagnostic count fits u64")
            .to_le_bytes(),
    );
    for diagnostic in diagnostics {
        append_length_prefixed(&mut stream, diagnostic.code.as_bytes());
        stream.push(ERROR_SEVERITY);
        append_length_prefixed(&mut stream, diagnostic.source.document_key.as_bytes());
        stream.extend_from_slice(&diagnostic.source.start_line.to_le_bytes());
        stream.extend_from_slice(&diagnostic.source.start_column.to_le_bytes());
        stream.extend_from_slice(&diagnostic.source.end_line.to_le_bytes());
        stream.extend_from_slice(&diagnostic.source.end_column.to_le_bytes());
        append_length_prefixed(&mut stream, &diagnostic.typed_payload);
    }
    lower_hex(&Sha256::digest(stream))
}

fn append_length_prefixed(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(
        &u32::try_from(bytes.len())
            .expect("diagnostic field length fits u32")
            .to_le_bytes(),
    );
    output.extend_from_slice(bytes);
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stream_contains_frozen_header_and_zero_count() {
        let mut bytes = Vec::from(DIAGNOSTIC_STREAM_DOMAIN);
        bytes.extend_from_slice(&DIAGNOSTIC_STREAM_VERSION.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        assert_eq!(empty_diagnostic_digest(), lower_hex(&Sha256::digest(bytes)));
    }

    #[test]
    fn limit_payload_binds_dimension_limit_and_observed_value() {
        let baseline = limit_exceeded_diagnostic_digest("E", 3, 10, 11);
        assert_ne!(baseline, limit_exceeded_diagnostic_digest("E", 4, 10, 11));
        assert_ne!(baseline, limit_exceeded_diagnostic_digest("E", 3, 9, 11));
        assert_ne!(baseline, limit_exceeded_diagnostic_digest("E", 3, 10, 12));
    }
}
