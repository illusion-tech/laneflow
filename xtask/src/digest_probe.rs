//! #794 摘要失配转储的离线页指纹判读。
//!
//! `laneflow-format` 读回校验失配时，失配点只剩失真字节（`.bin`）与探针元数据
//! （同名 `.probe`：entry、offset、长度、期望/实际摘要），原始字节已不在手。本命令
//! 以健康的确定性产物为金样本：目录项里的期望摘要原样落在产物中，据此反推 chunk
//! 在文件中的位置，并以摘要自校验金样本取对；随后按 4 KiB 文件页逐页对照，输出
//! `validation-matrix.md` §8 失真指纹所需的首个分歧偏移、页对齐、整页替换、零页与
//! 页整数倍位移，并给出判读结论。只读诊断，不改变任何失败语义。

use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

const PAGE: u64 = 4_096;
/// 位移搜索范围（页）；归档实测最大 24 页。
const MAX_SHIFT_PAGES: i64 = 64;
/// 分块节前导与目录项内摘要字段的偏移，与 `laneflow-static-contract::portable`
/// 的 `CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH` 及目录项布局一致。
const SECTION_PREAMBLE: u64 = 16;
const ENTRY_DIGEST_OFFSET: u64 = 40;
/// 整页替换判定的页内分段数。
const REPLACEMENT_SEGMENTS: usize = 16;
/// 近零页：非零字节至多这么多。归档与浸泡实测的写丢失页为 4,095 / 4,096 字节为零。
/// 两项阈值共同遵循 `docs/reference/validation-matrix.md` §8.1，且均包含边界。
const NEAR_ZERO_MAX_NONZERO: usize = 16;
/// 近零页还须有至少这么多原非零字节被清零，避免把零字节居多的页上的局部改动当作零页。
const NEAR_ZERO_MIN_CLEARED: usize = PAGE as usize / 16;
/// 报告逐页明细的上限，其余只计入汇总。
const MAX_PAGE_LINES: usize = 32;

const USAGE: &str = "用法: cargo +1.99.1 run --locked -p xtask -- classify-digest-probe --dump <转储.bin> --golden <健康产物文件或目录> [--golden ...]";

pub fn run(args: &[String]) -> Result<(), String> {
    let mut dump = None;
    let mut golden = Vec::new();
    let mut tokens = args.iter();
    while let Some(token) = tokens.next() {
        let value = tokens.next().ok_or(USAGE)?;
        match token.as_str() {
            "--dump" => dump = Some(PathBuf::from(value)),
            "--golden" => golden.push(PathBuf::from(value)),
            _ => return Err(USAGE.to_owned()),
        }
    }
    let dump = dump.ok_or(USAGE)?;
    if golden.is_empty() {
        return Err(USAGE.to_owned());
    }
    let probe = Probe::read(&dump)?;
    let corrupted =
        fs::read(&dump).map_err(|error| format!("读取转储 {} 失败: {error}", dump.display()))?;
    // 判读对象本身可能在疑似失真的存储上受损：先以旁文件记录的实际摘要核对转储。
    check_integrity(&probe, &corrupted)?;
    let mut files = Vec::new();
    for path in &golden {
        collect_files(path, &mut files)?;
    }
    for path in files {
        let bytes = fs::read(&path)
            .map_err(|error| format!("读取金样本 {} 失败: {error}", path.display()))?;
        if let Some(chunk_start) = locate(&bytes, &probe) {
            let report = analyze(&bytes, chunk_start, probe.len, &corrupted);
            println!("golden={} chunk_file_offset={chunk_start}", path.display());
            print!("{}", report.render(&probe, corrupted.len()));
            return Ok(());
        }
    }
    Err(format!(
        "金样本中找不到期望摘要 {} 对应的 chunk；确认金样本由同一配置与规模的健康运行生成",
        probe.expected_hex
    ))
}

/// 转储旁文件记录的探针现场。
#[derive(Debug)]
struct Probe {
    entry: u64,
    offset: u64,
    len: u64,
    expected: [u8; 32],
    expected_hex: String,
    /// 失配当时读回字节的摘要，用于核对转储未在落盘后改变。
    actual: [u8; 32],
    actual_hex: String,
}

impl Probe {
    fn read(dump: &Path) -> Result<Self, String> {
        let path = dump.with_extension("probe");
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("读取探针旁文件 {} 失败: {error}", path.display()))?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let field = |key: &str| {
            text.lines()
                .find_map(|line| {
                    line.strip_prefix(key)
                        .and_then(|rest| rest.strip_prefix('='))
                })
                .ok_or_else(|| format!("探针旁文件缺少 {key}"))
        };
        let number = |key: &str| -> Result<u64, String> {
            field(key)?
                .parse()
                .map_err(|_| format!("探针旁文件 {key} 不是整数"))
        };
        let expected_hex = field("expected")?.to_owned();
        let expected =
            parse_digest(&expected_hex).ok_or("探针旁文件 expected 不是 32 字节十六进制")?;
        let actual_hex = field("actual")?.to_owned();
        let actual = parse_digest(&actual_hex).ok_or("探针旁文件 actual 不是 32 字节十六进制")?;
        Ok(Self {
            entry: number("entry")?,
            offset: number("offset")?,
            len: number("len")?,
            expected,
            expected_hex,
            actual,
            actual_hex,
        })
    }
}

/// 转储摘要与旁文件 `actual` 不一致时不给出因果结论，按证据完整性失配单独报告。
fn check_integrity(probe: &Probe, dump: &[u8]) -> Result<(), String> {
    let digest: [u8; 32] = Sha256::digest(dump).into();
    if digest == probe.actual {
        return Ok(());
    }
    let dump_hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Err(format!(
        "verdict=evidence-mismatch 转储摘要 {dump_hex} 与旁文件 actual {} 不一致：转储在落盘后已改变，不作判读",
        probe.actual_hex
    ))
}

fn parse_digest(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(digest)
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)
            .map_err(|error| format!("读取目录 {} 失败: {error}", path.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for entry in entries {
            collect_files(&entry, files)?;
        }
    } else {
        files.push(path.to_path_buf());
    }
    Ok(())
}

/// 在金样本中按期望摘要定位目录项，反推 chunk 文件偏移并以摘要自校验。
fn locate(bytes: &[u8], probe: &Probe) -> Option<u64> {
    let back = SECTION_PREAMBLE + probe.entry + ENTRY_DIGEST_OFFSET;
    bytes
        .windows(32)
        .enumerate()
        .filter(|(_, window)| *window == probe.expected)
        .find_map(|(position, _)| {
            let section = (position as u64).checked_sub(back)?;
            let start = section.checked_add(probe.offset)?;
            let end = start.checked_add(probe.len)?;
            let chunk = bytes.get(usize::try_from(start).ok()?..usize::try_from(end).ok()?)?;
            (Sha256::digest(chunk).as_slice() == probe.expected).then_some(start)
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageKind {
    /// 失真页全零或近零（写丢失）。
    Zero,
    /// 失真页与金样本相距若干整页的另一页逐字节相同（页整数倍位移）。
    Shifted(i64),
    /// 差异分布覆盖整页（整页替换）。产物零字节多，替换页与原页常有大量字节巧合
    /// 相同，故按差异覆盖的页内分段而非差异字节数判定。
    Replaced,
    /// 差异只占页内一段：不符合页级失真指纹。
    Partial,
    /// chunk 首尾不满一页的残片：样本不足以判断整页形态，不作任何一方的证据。
    Fragment,
}

#[derive(Debug)]
struct PageFinding {
    /// 文件页序号（文件偏移 / 4096）。
    page: u64,
    kind: PageKind,
    differing: usize,
    compared: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Verdict {
    Identical,
    PageFingerprint,
    CodeSuspect,
    Inconclusive,
}

#[derive(Debug)]
struct Report {
    /// 首个分歧的文件偏移。
    first_diff: Option<u64>,
    chunk_start: u64,
    len_delta: i64,
    pages: Vec<PageFinding>,
    verdict: Verdict,
}

impl Report {
    fn render(&self, probe: &Probe, dump_len: usize) -> String {
        let mut text = format!(
            "entry={} offset={} len={} dump_len={dump_len} len_delta={}\n",
            probe.entry, probe.offset, probe.len, self.len_delta
        );
        match self.first_diff {
            Some(offset) => text.push_str(&format!(
                "first_diff chunk_offset={} file_offset={offset} in_page_offset={} page_aligned={}\n",
                offset - self.chunk_start,
                offset % PAGE,
                offset % PAGE == 0
            )),
            None => text.push_str("first_diff=none\n"),
        }
        for finding in self.pages.iter().take(MAX_PAGE_LINES) {
            text.push_str(&format!(
                "page={} file_offset={} kind={:?} differing={}/{}\n",
                finding.page,
                finding.page * PAGE,
                finding.kind,
                finding.differing,
                finding.compared
            ));
        }
        if self.pages.len() > MAX_PAGE_LINES {
            text.push_str(&format!(
                "pages_omitted={}\n",
                self.pages.len() - MAX_PAGE_LINES
            ));
        }
        let count = |wanted: fn(PageKind) -> bool| {
            self.pages
                .iter()
                .filter(|finding| wanted(finding.kind))
                .count()
        };
        text.push_str(&format!(
            "summary differing_pages={} replaced={} zero={} shifted={} partial={} fragment={}\n",
            self.pages.len(),
            count(|kind| kind == PageKind::Replaced),
            count(|kind| kind == PageKind::Zero),
            count(|kind| matches!(kind, PageKind::Shifted(_))),
            count(|kind| kind == PageKind::Partial),
            count(|kind| kind == PageKind::Fragment),
        ));
        text.push_str(match self.verdict {
            Verdict::Identical => "verdict=identical 转储与金样本一致：失真不在转储字节中（读回瞬时失真或转储前已恢复）\n",
            Verdict::PageFingerprint => "verdict=page-fingerprint 符合页级失真指纹：按环境失真裁决，并以独立写-读浸泡对照确认\n",
            Verdict::CodeSuspect => "verdict=code-suspect 存在非页对齐或页内局部差异：不符合页级失真指纹，转代码嫌疑\n",
            Verdict::Inconclusive => "verdict=inconclusive 差异只落在 chunk 首尾不满一页的残片上：证据不足，保持待裁决\n",
        });
        text
    }
}

/// 按 4 KiB 文件页对照转储与金样本。`chunk_start` 是 chunk 在金样本文件中的偏移，
/// 页网格以文件偏移为准（staged 产物经 mmap 读回，文件页即内存页）。
fn analyze(golden: &[u8], chunk_start: u64, chunk_len: u64, dump: &[u8]) -> Report {
    let start = chunk_start as usize;
    let compared_len = dump.len().min(chunk_len as usize);
    let expected = &golden[start..start + compared_len];
    let first_diff = expected
        .iter()
        .zip(dump)
        .position(|(left, right)| left != right)
        .map(|index| chunk_start + index as u64);
    let mut pages = Vec::new();
    if compared_len > 0 {
        let first_page = chunk_start / PAGE;
        let last_page = (chunk_start + compared_len as u64 - 1) / PAGE;
        for page in first_page..=last_page {
            let from = (page * PAGE).max(chunk_start);
            let to = ((page + 1) * PAGE).min(chunk_start + compared_len as u64);
            let range = (from - chunk_start) as usize..(to - chunk_start) as usize;
            let (want, got) = (&expected[range.clone()], &dump[range]);
            let differing = want
                .iter()
                .zip(got)
                .filter(|(left, right)| left != right)
                .count();
            if differing == 0 {
                continue;
            }
            let kind = classify_page(golden, page, from, want, got);
            pages.push(PageFinding {
                page,
                kind,
                differing,
                compared: want.len(),
            });
        }
    }
    let len_delta = dump.len() as i64 - chunk_len as i64;
    let verdict = if first_diff.is_none() && len_delta == 0 {
        Verdict::Identical
    } else if len_delta != 0
        || pages
            .iter()
            .any(|finding| finding.kind == PageKind::Partial)
    {
        Verdict::CodeSuspect
    } else if pages
        .iter()
        .any(|finding| finding.kind != PageKind::Fragment)
    {
        Verdict::PageFingerprint
    } else {
        Verdict::Inconclusive
    };
    Report {
        first_diff,
        chunk_start,
        len_delta,
        pages,
        verdict,
    }
}

fn classify_page(golden: &[u8], page: u64, from: u64, want: &[u8], got: &[u8]) -> PageKind {
    // 页级形态只对完整页成立；首尾残片再小也可能凑出零页或「铺满」。
    if from != page * PAGE || got.len() != PAGE as usize {
        return PageKind::Fragment;
    }
    if got.iter().all(|byte| *byte == 0) {
        return PageKind::Zero;
    }
    let found = (1..=MAX_SHIFT_PAGES).flat_map(|k| [k, -k]).find(|k| {
        let Some(source) = (page as i64).checked_add(*k).filter(|source| *source >= 0) else {
            return false;
        };
        let begin = source as usize * PAGE as usize;
        golden.get(begin..begin + PAGE as usize) == Some(got)
    });
    if let Some(shift) = found {
        return PageKind::Shifted(shift);
    }
    // 写丢失页常残留个别非零字节；原页零段多时差异只覆盖部分分段，不按零页认会落入
    // Partial。须同时有成段原内容被清零，零字节居多的页上的局部改动仍归 Partial。
    let nonzero = got.iter().filter(|byte| **byte != 0).count();
    let cleared = want
        .iter()
        .zip(got)
        .filter(|(left, right)| **left != 0 && **right == 0)
        .count();
    if nonzero <= NEAR_ZERO_MAX_NONZERO && cleared >= NEAR_ZERO_MIN_CLEARED {
        return PageKind::Zero;
    }
    // 页均分为 REPLACEMENT_SEGMENTS 段，差异须覆盖其中至少 3/4 才算整页替换；
    // 只看首尾差异会把页两端的零散翻转误判为替换。
    let segment = want.len().div_ceil(REPLACEMENT_SEGMENTS).max(1);
    let (hit, total) = want
        .chunks(segment)
        .zip(got.chunks(segment))
        .fold((0, 0), |(hit, total), (left, right)| {
            (hit + usize::from(left != right), total + 1)
        });
    if hit * 4 >= total * 3 {
        PageKind::Replaced
    } else {
        PageKind::Partial
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 确定性伪随机字节（线性同余），保证各页内容互不相同。
    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u8
            })
            .collect()
    }

    /// 金样本：`prefix` 字节前缀 + 单目录项分块节（chunk 位于节内 88 处）+ 尾随页。
    fn golden(prefix: usize, chunk_len: usize) -> (Vec<u8>, Probe, u64) {
        let chunk = noise(chunk_len, 7);
        let digest: [u8; 32] = Sha256::digest(&chunk).into();
        let mut bytes = noise(prefix, 3);
        let section = bytes.len();
        bytes.extend_from_slice(&[0; 16 + 72]);
        bytes[section + 16 + 40..section + 16 + 72].copy_from_slice(&digest);
        bytes.extend_from_slice(&chunk);
        bytes.extend_from_slice(&noise(40 * PAGE as usize, 11));
        let probe = Probe {
            entry: 0,
            offset: 88,
            len: chunk_len as u64,
            expected: digest,
            expected_hex: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
            // 判读测试直接调用 analyze，不经完整性核对；完整性另有专项测试。
            actual: [0; 32],
            actual_hex: "00".repeat(32),
        };
        (bytes, probe, (section + 88) as u64)
    }

    /// chunk 从文件页 1 开始（前缀 4096-88），覆盖 30 个整页。
    fn aligned() -> (Vec<u8>, Probe, u64) {
        golden(PAGE as usize - 88, 30 * PAGE as usize)
    }

    fn dump_of(bytes: &[u8], start: u64, len: u64) -> Vec<u8> {
        bytes[start as usize..(start + len) as usize].to_vec()
    }

    fn page_mut(dump: &mut [u8], start: u64, file_page: u64) -> &mut [u8] {
        let at = (file_page * PAGE - start) as usize;
        &mut dump[at..at + PAGE as usize]
    }

    #[test]
    fn locates_chunk_by_directory_digest_and_verifies_it() {
        let (bytes, probe, start) = golden(123, 9_000);
        assert_eq!(locate(&bytes, &probe), Some(start));
        let mut wrong = probe;
        wrong.offset += 1;
        assert_eq!(locate(&bytes, &wrong), None);
    }

    #[test]
    fn parses_probe_sidecar() {
        let hex = "ab".repeat(32);
        let probe = Probe::parse(&format!(
            "entry=72\noffset=290\nlen=5110968\nexpected={hex}\nactual={hex}\n"
        ))
        .unwrap();
        assert_eq!((probe.entry, probe.offset, probe.len), (72, 290, 5_110_968));
        assert_eq!(probe.expected, [0xab; 32]);
        assert!(Probe::parse("entry=0\n").is_err());
    }

    #[test]
    fn identical_dump_is_not_a_fingerprint() {
        let (bytes, probe, start) = aligned();
        let report = analyze(&bytes, start, probe.len, &dump_of(&bytes, start, probe.len));
        assert_eq!(report.verdict, Verdict::Identical);
    }

    #[test]
    fn whole_page_substitution_is_a_page_fingerprint() {
        let (bytes, probe, start) = aligned();
        let mut dump = dump_of(&bytes, start, probe.len);
        page_mut(&mut dump, start, 5).copy_from_slice(&noise(PAGE as usize, 99));
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.first_diff, Some(5 * PAGE));
        assert_eq!(report.pages.len(), 1);
        assert_eq!(report.pages[0].kind, PageKind::Replaced);
        assert_eq!(report.verdict, Verdict::PageFingerprint);
    }

    #[test]
    fn page_multiple_shifts_are_found_in_both_directions() {
        let (bytes, probe, start) = aligned();
        for shift in [1_i64, -1, 24, -3] {
            let mut dump = dump_of(&bytes, start, probe.len);
            let target = 10_u64;
            let source = (target as i64 + shift) as usize * PAGE as usize;
            page_mut(&mut dump, start, target)
                .copy_from_slice(&bytes[source..source + PAGE as usize]);
            let report = analyze(&bytes, start, probe.len, &dump);
            assert_eq!(
                report.pages[0].kind,
                PageKind::Shifted(shift),
                "shift {shift}"
            );
            assert_eq!(report.verdict, Verdict::PageFingerprint);
        }
    }

    #[test]
    fn zero_page_is_a_page_fingerprint() {
        let (bytes, probe, start) = aligned();
        let mut dump = dump_of(&bytes, start, probe.len);
        page_mut(&mut dump, start, 7).fill(0);
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.pages[0].kind, PageKind::Zero);
        assert_eq!(report.verdict, Verdict::PageFingerprint);
    }

    #[test]
    fn near_zero_write_loss_page_is_a_zero_page() {
        // 浸泡实测形态：原页后半为零段，失真页 4,095 字节为零、残留 1 个非零字节。
        // 差异只覆盖前半分段，若不认近零会落入 Partial。
        let (mut bytes, probe, start) = aligned();
        let page = 8 * PAGE as usize;
        bytes[page + PAGE as usize / 2..page + PAGE as usize].fill(0);
        let mut dump = dump_of(&bytes, start, probe.len);
        let lost = page_mut(&mut dump, start, 8);
        lost.fill(0);
        lost[3_000] = 0x5a;
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.pages[0].kind, PageKind::Zero);
        assert_eq!(report.verdict, Verdict::PageFingerprint);

        // 原页满载时同样成立。
        let mut dump = dump_of(&bytes, start, probe.len);
        let lost = page_mut(&mut dump, start, 7);
        lost.fill(0);
        lost[100] = 0x01;
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.pages[0].kind, PageKind::Zero);
    }

    #[test]
    fn local_clearing_on_a_mostly_zero_page_is_code_suspect() {
        // 原页只有一小段非零，局部清零后虽近零，但被清零的原内容不成段，不按写丢失认。
        let (mut bytes, probe, start) = aligned();
        let page = 8 * PAGE as usize;
        bytes[page..page + PAGE as usize].fill(0);
        bytes[page + 1_000..page + 1_040].fill(0x33);
        let mut dump = dump_of(&bytes, start, probe.len);
        page_mut(&mut dump, start, 8)[1_000..1_030].fill(0);
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.pages[0].kind, PageKind::Partial);
        assert_eq!(report.verdict, Verdict::CodeSuspect);
    }

    #[test]
    fn near_zero_requires_both_thresholds() {
        for (nonzero, cleared, expected) in [
            (15, 255, PageKind::Partial),
            (15, 256, PageKind::Zero),
            (15, 257, PageKind::Zero),
            (16, 255, PageKind::Partial),
            (16, 256, PageKind::Zero),
            (16, 257, PageKind::Zero),
            (17, 255, PageKind::Partial),
            (17, 256, PageKind::Partial),
            (17, 257, PageKind::Partial),
        ] {
            let (mut bytes, probe, start) = aligned();
            let page = 8 * PAGE as usize;
            bytes[page..page + PAGE as usize].fill(0);
            bytes[page..page + cleared].fill(0x33);
            let mut dump = dump_of(&bytes, start, probe.len);
            let lost = page_mut(&mut dump, start, 8);
            lost.fill(0);
            lost[3_000..3_000 + nonzero].fill(0x5a);
            let report = analyze(&bytes, start, probe.len, &dump);
            assert_eq!(report.pages.len(), 1);
            assert_eq!(
                report.pages[0].kind, expected,
                "nonzero {nonzero}, cleared {cleared}"
            );
            assert_eq!(
                report.verdict,
                if expected == PageKind::Zero {
                    Verdict::PageFingerprint
                } else {
                    Verdict::CodeSuspect
                }
            );
        }
    }

    #[test]
    fn near_zero_exact_donor_match_precedes_zero_classification() {
        for donor in [7, 9] {
            let (mut bytes, probe, start) = aligned();
            let page = 8 * PAGE as usize;
            bytes[page..page + PAGE as usize].fill(0);
            bytes[page..page + 256].fill(0x33);
            let donor_start = donor * PAGE as usize;
            bytes[donor_start..donor_start + PAGE as usize].fill(0);
            bytes[donor_start + 3_000..donor_start + 3_016].fill(0x5a);
            let mut dump = dump_of(&bytes, start, probe.len);
            page_mut(&mut dump, start, 8)
                .copy_from_slice(&bytes[donor_start..donor_start + PAGE as usize]);
            let report = analyze(&bytes, start, probe.len, &dump);
            assert_eq!(report.pages.len(), 1);
            assert_eq!(report.pages[0].kind, PageKind::Shifted(donor as i64 - 8));
            assert_eq!(report.verdict, Verdict::PageFingerprint);
        }
    }

    #[test]
    fn near_zero_fragment_is_not_page_evidence() {
        let want = vec![0x33; PAGE as usize - 1];
        let mut got = vec![0; want.len()];
        got[100] = 0x5a;
        assert_eq!(classify_page(&want, 0, 0, &want, &got), PageKind::Fragment);
    }

    #[test]
    fn single_byte_flip_inside_a_page_is_code_suspect() {
        let (bytes, probe, start) = aligned();
        let mut dump = dump_of(&bytes, start, probe.len);
        let at = (12 * PAGE + 1_234 - start) as usize;
        dump[at] ^= 0x40;
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.first_diff, Some(12 * PAGE + 1_234));
        assert_eq!(report.pages[0].kind, PageKind::Partial);
        assert_eq!(report.verdict, Verdict::CodeSuspect);
        assert!(
            report
                .render(&probe, dump.len())
                .contains("page_aligned=false")
        );
    }

    #[test]
    fn page_grid_follows_file_offsets_when_chunk_is_unaligned() {
        // chunk 起点不在页边界：失真按文件页落在 chunk 内部的页边界上。
        let (bytes, probe, start) = golden(1_000, 20 * PAGE as usize);
        assert_ne!(start % PAGE, 0);
        let mut dump = dump_of(&bytes, start, probe.len);
        page_mut(&mut dump, start, 6).fill(0);
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.first_diff.map(|offset| offset % PAGE), Some(0));
        assert_eq!(report.pages[0].page, 6);
        assert_eq!(report.verdict, Verdict::PageFingerprint);
    }

    /// 零字节居多的页：每 16 字节才有一个非零字节。
    fn sparse(seed: u64) -> Vec<u8> {
        noise(PAGE as usize, seed)
            .into_iter()
            .enumerate()
            .map(|(index, byte)| if index % 16 == 0 { byte | 1 } else { 0 })
            .collect()
    }

    #[test]
    fn substitution_between_sparse_pages_is_still_a_replacement() {
        // 替换页与原页大量字节巧合相同（同为零），差异字节远少于 3/4 页。
        let (mut bytes, probe, start) = aligned();
        bytes[8 * PAGE as usize..9 * PAGE as usize].copy_from_slice(&sparse(1));
        let chunk = &bytes[start as usize..(start + probe.len) as usize];
        let digest: [u8; 32] = Sha256::digest(chunk).into();
        let probe = Probe {
            expected: digest,
            ..probe
        };
        let mut dump = dump_of(&bytes, start, probe.len);
        page_mut(&mut dump, start, 8).copy_from_slice(&sparse(2));
        let report = analyze(&bytes, start, probe.len, &dump);
        assert!(report.pages[0].differing < PAGE as usize / 4);
        assert_eq!(report.pages[0].kind, PageKind::Replaced);
        assert_eq!(report.verdict, Verdict::PageFingerprint);
    }

    #[test]
    fn contiguous_overwrite_inside_a_page_is_code_suspect() {
        let (bytes, probe, start) = aligned();
        let mut dump = dump_of(&bytes, start, probe.len);
        let at = (15 * PAGE + 1_000 - start) as usize;
        dump[at..at + 64].copy_from_slice(&noise(64, 5));
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.pages[0].kind, PageKind::Partial);
        assert_eq!(report.verdict, Verdict::CodeSuspect);
    }

    #[test]
    fn isolated_flips_at_both_page_ends_are_not_a_replacement() {
        // 审阅反例：页首、页尾各一个字节翻转，内部未变。
        let (bytes, probe, start) = aligned();
        let mut dump = dump_of(&bytes, start, probe.len);
        let page = (9 * PAGE - start) as usize;
        dump[page] ^= 0x01;
        dump[page + PAGE as usize - 1] ^= 0x80;
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.pages[0].differing, 2);
        assert_eq!(report.pages[0].kind, PageKind::Partial);
        assert_eq!(report.verdict, Verdict::CodeSuspect);
    }

    #[test]
    fn tail_fragment_alone_is_inconclusive() {
        // 审阅反例：页对齐的 4,097 字节 chunk，只翻转最后一个字节（落在 1 字节残片上）。
        let (bytes, probe, start) = golden(PAGE as usize - 88, PAGE as usize + 1);
        assert_eq!(start % PAGE, 0);
        let mut dump = dump_of(&bytes, start, probe.len);
        *dump.last_mut().unwrap() ^= 0x01;
        let report = analyze(&bytes, start, probe.len, &dump);
        assert_eq!(report.pages.len(), 1);
        assert_eq!(report.pages[0].kind, PageKind::Fragment);
        assert_eq!(report.verdict, Verdict::Inconclusive);

        // 残片上全零同样不足以判零页。
        let mut dump = dump_of(&bytes, start, probe.len);
        *dump.last_mut().unwrap() = 0;
        if dump != dump_of(&bytes, start, probe.len) {
            let report = analyze(&bytes, start, probe.len, &dump);
            assert_eq!(report.pages[0].kind, PageKind::Fragment);
        }
    }

    #[test]
    fn fragment_does_not_mask_full_page_fingerprint() {
        let (bytes, probe, start) = golden(PAGE as usize - 88, 3 * PAGE as usize + 100);
        let mut dump = dump_of(&bytes, start, probe.len);
        page_mut(&mut dump, start, 2).fill(0);
        *dump.last_mut().unwrap() ^= 0x01;
        let report = analyze(&bytes, start, probe.len, &dump);
        let kinds: Vec<_> = report.pages.iter().map(|finding| finding.kind).collect();
        assert_eq!(kinds, [PageKind::Zero, PageKind::Fragment]);
        assert_eq!(report.verdict, Verdict::PageFingerprint);
    }

    #[test]
    fn dump_must_match_recorded_actual_digest() {
        let (bytes, mut probe, start) = aligned();
        let mut dump = dump_of(&bytes, start, probe.len);
        page_mut(&mut dump, start, 4).fill(0);
        probe.actual = Sha256::digest(&dump).into();
        probe.actual_hex = probe
            .actual
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert!(check_integrity(&probe, &dump).is_ok());
        // 审阅反例：旁文件记录的是零页失真，落盘后的转储却变成单字节翻转。
        let mut altered = dump_of(&bytes, start, probe.len);
        altered[100] ^= 0x01;
        let error = check_integrity(&probe, &altered).unwrap_err();
        assert!(error.starts_with("verdict=evidence-mismatch"), "{error}");
    }
}
