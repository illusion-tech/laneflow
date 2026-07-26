//! Deterministic uncompressed POSIX ustar writer (§8).

use crate::{Error, Result};

const BLOCK: usize = 512;
const MODE_REGULAR: u64 = 0o644;

/// One regular-file member for a deterministic tar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TarMember {
    pub path: String,
    pub contents: Vec<u8>,
}

/// Build an uncompressed ustar archive.
///
/// Contract: path ordered by raw UTF-8 bytes; `mtime=0`; `uid=gid=0`; empty
/// owner/group names; regular-file mode `0644`; trailing two zero blocks.
pub fn write_deterministic_ustar(members: &[TarMember]) -> Result<Vec<u8>> {
    let mut ordered = members.to_vec();
    ordered.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    for window in ordered.windows(2) {
        if window[0].path == window[1].path {
            return Err(Error::SumoModel(format!(
                "duplicate tar member path {:?}",
                window[0].path
            )));
        }
    }

    let mut out = Vec::new();
    for member in &ordered {
        validate_path(&member.path)?;
        out.extend_from_slice(&ustar_header(&member.path, member.contents.len())?);
        out.extend_from_slice(&member.contents);
        let pad = (BLOCK - (member.contents.len() % BLOCK)) % BLOCK;
        out.resize(out.len() + pad, 0);
    }
    out.resize(out.len() + BLOCK * 2, 0);
    Ok(out)
}

fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(Error::SumoModel("tar member path must not be empty".to_owned()));
    }
    if path.as_bytes().contains(&0) {
        return Err(Error::SumoModel(format!(
            "tar member path contains NUL: {path:?}"
        )));
    }
    if path.starts_with('/') || path.contains('\\') {
        return Err(Error::SumoModel(format!(
            "tar member path must be relative POSIX: {path:?}"
        )));
    }
    if path.len() > 100 {
        return Err(Error::SumoModel(format!(
            "tar member path exceeds ustar 100-byte name field: {path:?}"
        )));
    }
    Ok(())
}

fn ustar_header(path: &str, size: usize) -> Result<[u8; BLOCK]> {
    let mut header = [0_u8; BLOCK];
    write_bytes(&mut header[0..100], path.as_bytes());
    write_octal(&mut header[100..108], MODE_REGULAR, 7)?;
    write_octal(&mut header[108..116], 0, 7)?; // uid
    write_octal(&mut header[116..124], 0, 7)?; // gid
    write_octal(&mut header[124..136], size as u64, 11)?;
    write_octal(&mut header[136..148], 0, 11)?; // mtime
    // checksum placeholder filled with spaces while summing
    header[148..156].fill(b' ');
    header[156] = b'0'; // regular file
    // linkname left zero
    write_bytes(&mut header[257..263], b"ustar\0");
    write_bytes(&mut header[263..265], b"00");
    // uname / gname left empty (zeros)
    let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    write_octal(&mut header[148..156], u64::from(sum), 6)?;
    header[154] = 0;
    header[155] = b' ';
    Ok(header)
}

fn write_bytes(dest: &mut [u8], bytes: &[u8]) {
    dest[..bytes.len()].copy_from_slice(bytes);
}

fn write_octal(dest: &mut [u8], value: u64, digits: usize) -> Result<()> {
    // classic tar: `digits` octal digits, then NUL (field may be longer)
    if dest.len() < digits + 1 {
        return Err(Error::SumoModel(
            "internal tar octal field too small".to_owned(),
        ));
    }
    let encoded = format!("{value:0digits$o}");
    if encoded.len() > digits {
        return Err(Error::SumoModel(format!(
            "tar octal value {value} does not fit in {digits} digits"
        )));
    }
    let start = digits - encoded.len();
    for slot in dest.iter_mut().take(start) {
        *slot = b'0';
    }
    write_bytes(&mut dest[start..digits], encoded.as_bytes());
    dest[digits] = 0;
    for slot in dest.iter_mut().skip(digits + 1) {
        *slot = 0;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_sorted_by_utf8_bytes_and_byte_identical() {
        let members = [
            TarMember {
                path: "b.txt".to_owned(),
                contents: b"b".to_vec(),
            },
            TarMember {
                path: "a.txt".to_owned(),
                contents: b"a".to_vec(),
            },
        ];
        let first = write_deterministic_ustar(&members).expect("tar");
        let second = write_deterministic_ustar(&members).expect("tar");
        assert_eq!(first, second);
        // path "a.txt" header precedes "b.txt"
        let a_pos = find_name(&first, "a.txt");
        let b_pos = find_name(&first, "b.txt");
        assert!(a_pos < b_pos);
        assert_eq!(first.len() % BLOCK, 0);
    }

    fn find_name(archive: &[u8], name: &str) -> usize {
        archive
            .windows(name.len())
            .position(|window| window == name.as_bytes())
            .expect("name present")
    }
}
