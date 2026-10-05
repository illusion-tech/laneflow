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

/// #253 Q3：tar member 路径必须是规范相对路径——拒绝绝对路径与 `..` /
/// `.` / 空组件（防写出解压目录逃逸）。
fn validate_member_path(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return Err(Error::SumoModel(format!(
            "tar member path {path:?} is not a relative path"
        )));
    }
    if path
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(Error::SumoModel(format!(
            "tar member path {path:?} contains a non-canonical component"
        )));
    }
    Ok(())
}

/// Build an uncompressed ustar archive.
///
/// Contract: path ordered by raw UTF-8 bytes; `mtime=0`; `uid=gid=0`; empty
/// owner/group names; regular-file mode `0644`; trailing two zero blocks.
pub fn write_deterministic_ustar(members: &[TarMember]) -> Result<Vec<u8>> {
    for member in members {
        validate_member_path(&member.path)?;
    }
    // 排序引用而非克隆成员——pinned source 载荷约 143 MB，深拷贝会在
    // 原件与输出缓冲之外再造一份完整副本。
    let mut ordered: Vec<&TarMember> = members.iter().collect();
    ordered.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    for window in ordered.windows(2) {
        if window[0].path == window[1].path {
            return Err(Error::SumoModel(format!(
                "duplicate tar member path {:?}",
                window[0].path
            )));
        }
    }

    // 预分配精确容量：成员载荷（约 143 MB）常驻期间，空 Vec 的摊销增长
    // 会在 realloc 瞬间并存新旧两份归档缓冲。先算 checked 总量一次到位。
    let mut total = BLOCK * 2;
    for member in &ordered {
        total = [
            BLOCK,
            member.contents.len(),
            (BLOCK - member.contents.len() % BLOCK) % BLOCK,
        ]
        .into_iter()
        .try_fold(total, usize::checked_add)
        .ok_or_else(|| Error::SumoModel("tar archive size overflowed usize".to_owned()))?;
    }
    let mut out = Vec::with_capacity(total);
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
        return Err(Error::SumoModel(
            "tar member path must not be empty".to_owned(),
        ));
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

    #[test]
    fn non_canonical_member_paths_fail_closed() {
        // #253 Q3：.. / . / 空组件 / 绝对路径全部拒绝（防写出解压目录逃逸）。
        for bad in ["../evil", "a/../../evil", "./x", "a//b", "/abs", "a/./b"] {
            let result = write_deterministic_ustar(&[TarMember {
                path: bad.to_owned(),
                contents: b"x".to_vec(),
            }]);
            assert!(result.is_err(), "path {bad:?} must be rejected");
        }
        assert!(
            write_deterministic_ustar(&[TarMember {
                path: "a/b.txt".to_owned(),
                contents: b"x".to_vec(),
            }])
            .is_ok()
        );
    }

    fn find_name(archive: &[u8], name: &str) -> usize {
        archive
            .windows(name.len())
            .position(|window| window == name.as_bytes())
            .expect("name present")
    }
}
