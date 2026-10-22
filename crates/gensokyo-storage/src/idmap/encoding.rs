//! 身份归一化与定长整数编码，逐条对齐 Go `idmap/map_service.go`。

/// 归一化后的身份：`key` 写正表，`raw` 写反表与 `last_seen`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedIdentity {
    /// `identity_to_vuin` 的 key。
    pub(crate) key: String,
    /// 反查与 `last_seen` 返回的原始身份串（Go `identity.Raw`）。
    pub(crate) raw: String,
}

/// Go `ididentity.IsOpenID`：长度 32 即视为 OpenID。
pub(crate) const OPENID_LEN: usize = 32;

/// 归一化身份，等价 Go `normalizeIdentity`。
///
/// - 先按 `uin` 隔离前缀剥离（Go `stripUinPrefix` / `uinPrefix`）；
/// - `rUIN-<平台>-<序号>-<id>` 形式走 [`parse_ruin`]；
/// - 32 字节的串视为 OpenID，key 为 `openid:QQ:<app_id>:<raw>`；
/// - 其余 key 为 `raw:<raw>`。
pub(crate) fn normalize_identity(raw: &str, app_id: &str, uin: Option<&str>) -> NormalizedIdentity {
    let stripped = strip_uin_prefix(raw, uin);
    if let Some(identity) = parse_ruin(&stripped) {
        return identity;
    }
    if stripped.len() == OPENID_LEN {
        return NormalizedIdentity {
            key: format!("openid:QQ:{app_id}:{stripped}"),
            raw: stripped,
        };
    }
    NormalizedIdentity {
        key: format!("raw:{stripped}"),
        raw: stripped,
    }
}

/// Go `stripUinPrefix`：仅当配置了 `uin` 且串以 `"<uin>:"` 开头时剥离。
fn strip_uin_prefix(raw: &str, uin: Option<&str>) -> String {
    match uin {
        Some(uin) if !uin.is_empty() => {
            let prefix = format!("{uin}:");
            raw.strip_prefix(prefix.as_str()).unwrap_or(raw).to_owned()
        }
        _ => raw.to_owned(),
    }
}

/// Go `parseRUIN`：`rUIN-<平台>-<序号>-<id>`，`QQ` 平台的非数字 id 先做 base32 解码。
fn parse_ruin(raw: &str) -> Option<NormalizedIdentity> {
    let head = raw.get(..5)?;
    if !head.eq_ignore_ascii_case("ruin-") {
        return None;
    }
    let parts: Vec<&str> = raw.splitn(4, '-').collect();
    if parts.len() != 4 {
        return None;
    }
    let platform = parts[1];
    let index = parts[2];
    let mut id = parts[3].to_owned();
    if platform == "QQ"
        && !is_ascii_digits(&id)
        && let Some(decoded) = decode_base32(&id)
    {
        id = decoded;
    }
    Some(NormalizedIdentity {
        key: format!("ruin:{platform}:{index}:{id}"),
        raw: format!("rUIN-{platform}-{index}-{id}"),
    })
}

/// Go `isDigits`（本 crate 只认 ASCII 数字；Go 的 `unicode.IsDigit` 会接受阿拉伯数字等，
/// 见 crate 文档的差异说明）。
fn is_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// Go `decodeBase32ID`：RFC 4648 标准字母表、无填充、大小写不敏感。
///
/// 除字母表外，校验与 Go `base32.StdEncoding.WithPadding(NoPadding)` 一致：
/// 长度模 8 必须是 `0/2/4/5/7`，且尾随比特必须为 0；解码结果还必须非空且全为
/// ASCII 数字。任一条不满足返回 `None`，调用方保留原始 id（不报错）。
fn decode_base32(value: &str) -> Option<String> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    // Go 的 base32 解码器对 len%8 ∈ {1,3,6} 直接返回 CorruptInputError。
    if !matches!(value.len() % 8, 0 | 2 | 4 | 5 | 7) {
        return None;
    }

    let mut out = Vec::with_capacity(value.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in value.bytes() {
        let byte = byte.to_ascii_uppercase();
        let digit = ALPHABET.iter().position(|&a| a == byte)?;
        buffer = (buffer << 5) | u32::try_from(digit).ok()?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    // 尾随比特非 0 属于 Go 会拒绝的非规范编码（如 "GZ" 解出 "6" 但残位为 01）。
    if buffer & ((1u32 << bits) - 1) != 0 {
        return None;
    }
    let decoded = String::from_utf8(out).ok()?;
    if !is_ascii_digits(&decoded) {
        return None;
    }
    Some(decoded)
}

/// Go `putInt64`：8 字节大端。
pub(crate) fn encode_i64(value: i64) -> [u8; 8] {
    value.to_be_bytes()
}

/// Go `readInt64`：长度不是 8 字节时返回 `None`（调用方映射为 `Corrupt`）。
pub(crate) fn decode_i64(bytes: &[u8]) -> Option<i64> {
    <[u8; 8]>::try_from(bytes).ok().map(i64::from_be_bytes)
}

/// Go `vuinPrefix`：`"<十进制 vUIN>:"`。
pub(crate) fn vuin_prefix(vuin: i64) -> String {
    format!("{vuin}:")
}

/// Go `vuinIdentityKey`：`"<十进制 vUIN>:<identityKey>"`。
pub(crate) fn vuin_identity_key(vuin: i64, identity_key: &str) -> String {
    format!("{}{}", vuin_prefix(vuin), identity_key)
}
