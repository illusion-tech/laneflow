//! 文本 token 域比较助手（E0 #1140 交付的最小公共件）。
//!
//! 数字叶以原字面量比较（不做浮点再序列化），供浮点往返与候选位值回归消费。
//! 六来源基线包（pool.json / results-anchor / MANIFEST、`SIX_STREAMS_BASELINE_DIR`
//! 读取、旧拼池与锚文本设施）已随 #1411 退役；历史执行口径见冻结提交
//! （#1385 退役基线 3f80b65f 可查）。

// ===== 文本拼接原语（位忠实导出；数值字符串逐字保留，不做浮点再序列化）=====
// 背景（README 差异记录「导出路径有损再序列化（1 ULP）」）：经浮点解析/写出的
// 数值字符串存在 ±1 ULP 漂移（如 `10.220051765441895` → `10.220051765441896`）。
// 锚导出按字节从源文本部分，数字原文逐字保留。

/// 跳过一个 JSON 值（对象/数组/字符串/数字/字面量），`pos` 停在值后。
fn skip_json_value(text: &[u8], pos: &mut usize) {
    match text[*pos] {
        b'{' | b'[' => {
            let (open, close) = if text[*pos] == b'{' {
                (b'{', b'}')
            } else {
                (b'[', b']')
            };
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escape = false;
            loop {
                let byte = text[*pos];
                if in_string {
                    if escape {
                        escape = false;
                    } else if byte == b'\\' {
                        escape = true;
                    } else if byte == b'"' {
                        in_string = false;
                    }
                } else {
                    match byte {
                        b'"' => in_string = true,
                        b if b == open => depth += 1,
                        b if b == close => {
                            depth -= 1;
                            if depth == 0 {
                                *pos += 1;
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                *pos += 1;
            }
        }
        b'"' => {
            *pos += 1;
            let mut escape = false;
            loop {
                match text[*pos] {
                    b'\\' if !escape => escape = true,
                    b'"' if !escape => {
                        *pos += 1;
                        return;
                    }
                    _ => escape = false,
                }
                *pos += 1;
            }
        }
        _ => {
            while *pos < text.len()
                && !matches!(
                    text[*pos],
                    b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r'
                )
            {
                *pos += 1;
            }
        }
    }
}

/// JSON 文本叶 token（文本 token 域位级比较原语；serde_json 1.0.154 默认解析对 17 位
/// 十进制存在 1 ULP 误舍入，解析域位比较对忠实锚误报、对折叠对漏报——数值比较一律
/// 走本域）。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(in super::super) enum JsonLeafToken<'a> {
    /// 字符串字面量原文（含引号；键同此形态，逐字比较）。
    Str(&'a [u8]),
    /// 数字 token 原文。
    Number(&'a [u8]),
    /// `true`/`false`/`null` 原文。
    Literal(&'a [u8]),
}

/// 逐叶提取 JSON 文本 token（文档序；结构字符与空白忽略）。
pub(in super::super) fn json_leaf_tokens(text: &[u8]) -> Vec<JsonLeafToken<'_>> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        match text[pos] {
            b'"' => {
                let start = pos;
                skip_json_value(text, &mut pos);
                tokens.push(JsonLeafToken::Str(&text[start..pos]));
            }
            b'-' | b'0'..=b'9' => {
                let start = pos;
                skip_json_value(text, &mut pos);
                tokens.push(JsonLeafToken::Number(&text[start..pos]));
            }
            b't' | b'f' | b'n' => {
                let start = pos;
                skip_json_value(text, &mut pos);
                tokens.push(JsonLeafToken::Literal(&text[start..pos]));
            }
            _ => pos += 1,
        }
    }
    tokens
}

/// 文本 token 域位级比较（lead 插队规格）：数字 token 以 `f64::from_str`（标准库正确
/// 舍入）→ `to_bits` 比较（整数叶 u64 逐位）、字符串/字面量逐字比较；两文本叶流按文档
/// 序逐叶对齐，返回首个不匹配描述（`None` = 全等）。锚 vs 源切片、重算值 vs 锚统一走
/// 本域（全路复用）。
pub(in super::super) fn json_leaf_mismatch(expected: &str, actual: &str) -> Option<String> {
    let left = json_leaf_tokens(expected.as_bytes());
    let right = json_leaf_tokens(actual.as_bytes());
    if left.len() != right.len() {
        return Some(format!("leaf count {} vs {}", left.len(), right.len()));
    }
    for (index, (l, r)) in left.iter().zip(right.iter()).enumerate() {
        match (l, r) {
            (JsonLeafToken::Str(a), JsonLeafToken::Str(b))
            | (JsonLeafToken::Literal(a), JsonLeafToken::Literal(b)) => {
                if a != b {
                    return Some(format!(
                        "leaf {index}: string/literal {} vs {}",
                        String::from_utf8_lossy(a),
                        String::from_utf8_lossy(b)
                    ));
                }
            }
            (JsonLeafToken::Number(a), JsonLeafToken::Number(b)) => {
                if a == b {
                    continue;
                }
                let av = std::str::from_utf8(a).unwrap();
                let bv = std::str::from_utf8(b).unwrap();
                let floats = av.contains(['.', 'e', 'E']) || bv.contains(['.', 'e', 'E']);
                let (abits, bbits) = if floats {
                    (
                        av.parse::<f64>().unwrap().to_bits(),
                        bv.parse::<f64>().unwrap().to_bits(),
                    )
                } else if let (Ok(x), Ok(y)) = (av.parse::<u64>(), bv.parse::<u64>()) {
                    (x, y)
                } else {
                    (
                        av.parse::<f64>().unwrap().to_bits(),
                        bv.parse::<f64>().unwrap().to_bits(),
                    )
                };
                if abits != bbits {
                    return Some(format!(
                        "leaf {index}: number {av} (bits {abits:#018x}) vs {bv} (bits {bbits:#018x})"
                    ));
                }
            }
            _ => return Some(format!("leaf {index}: token kind mismatch")),
        }
    }
    None
}

#[test]
fn shared_baseline_text_token_domain_distinguishes_ulp_pair() {
    // 回归（lead 插队规格 3）：17 位十进制相邻对 …895/…896 在文本 token 域必须区分
    // （标准库正确舍入；serde_json 默认解析对二者误舍入折叠 → 漏报）。
    let faithful = "{\"x\":10.220051765441895}";
    assert_eq!(
        json_leaf_mismatch(faithful, "{\"x\":10.220051765441895}"),
        None
    );
    assert!(
        json_leaf_mismatch(faithful, "{\"x\":10.220051765441896}").is_some(),
        "ULP pair must be distinguished"
    );
    // 整数叶 u64 位逐位（program 位模式相邻对）。
    assert!(
        json_leaf_mismatch("{\"p\":4624643347766747136}", "{\"p\":4624643347766747137}").is_some()
    );
    assert_eq!(
        json_leaf_mismatch("{\"p\":4624643347766747136}", "{\"p\":4624643347766747136}"),
        None
    );
    // 字符串字面量逐字。
    assert!(json_leaf_mismatch("{\"s\":\"a\"}", "{\"s\":\"b\"}").is_some());
    assert_eq!(json_leaf_mismatch("{\"s\":\"a\"}", "{\"s\":\"a\"}"), None);
    // 与位忠实拼接衔接：文本 token 埲与逐字域在忠实文本上同判全等。
    let anchor_like = "{\"source\":{\"a\":1},\"k\":10.220051765441895}";
    assert_eq!(
        json_leaf_mismatch(
            "{\"source\":{\"a\":1},\"k\":10.220051765441895}",
            anchor_like
        ),
        None,
        "leaf domain ties to verbatim splice"
    );
}
