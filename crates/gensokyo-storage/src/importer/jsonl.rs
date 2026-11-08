//! v1 JSONL 的逐行解析与整文件校验（格式说明见 [`super`]）。

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::ImportError;

/// `meta.format` 的固定值。
const FORMAT: &str = "gensokyo-idmap";
/// 本导入器支持的唯一格式版本。
const FORMAT_VERSION: u32 = 1;

/// `meta` 行。
#[derive(Debug, Deserialize)]
struct MetaLine {
    format: String,
    version: u32,
    /// 导出侧的 vUIN 分配方式（`counter` / `hash`）；缺省时沿用本机 `hash_id` 配置。
    #[serde(default)]
    allocator: Option<Allocator>,
}

/// 导出侧的 vUIN 分配方式（Go `hash_id` 配置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Allocator {
    /// 顺序递增。
    Counter,
    /// MD5 派生（Go `GenerateRowID`）。
    Hash,
}

/// `identity` 行。
#[derive(Debug, Deserialize)]
pub(crate) struct IdentityLine {
    pub(crate) key: String,
    pub(crate) raw: String,
    pub(crate) vuin: i64,
}

/// `vuin_counter` / `msg_counter` 行（字段名以导出侧 `counter` 为准，`value` 作为兼容别名）。
#[derive(Debug, Deserialize)]
struct CounterLine {
    #[serde(alias = "value")]
    counter: i64,
}

/// `config` 行。
#[derive(Debug, Deserialize)]
pub(crate) struct ConfigLine {
    pub(crate) section: String,
    pub(crate) key: String,
    pub(crate) value: String,
}

/// `user_info` 行的线上形态：`value` 是任意 JSON（Go `structs.FriendData`）。
#[derive(Debug, Deserialize)]
struct UserInfoLine {
    key: String,
    value: Value,
}

/// `user_info` 行入库形态：值已紧凑序列化，写事务里不会再失败。
#[derive(Debug)]
pub(crate) struct UserInfoRecord {
    pub(crate) key: String,
    pub(crate) json: String,
}

/// `last_seen` 行。
#[derive(Debug, Deserialize)]
pub(crate) struct LastSeenLine {
    pub(crate) vuin: i64,
    pub(crate) raw: String,
}

/// `msg_id` 行（导出侧字段是 `real`，`expires` 可选）。
#[derive(Debug, Deserialize)]
pub(crate) struct MsgIdLine {
    #[serde(rename = "real")]
    pub(crate) msg_id: String,
    /// `virtual` 是 Rust 保留字，字段名走 `rename`。
    #[serde(rename = "virtual")]
    pub(crate) virtual_id: i64,
    /// 导出侧不带 TTL 时缺省：这类条目只写正反表，`expires_at` 留空（读时视为不过期）。
    #[serde(default)]
    pub(crate) expires: Option<i64>,
}

/// `latest` 行。
#[derive(Debug, Deserialize)]
pub(crate) struct LatestLine {
    pub(crate) group: String,
    pub(crate) user: String,
    pub(crate) msg_id: String,
    /// 同 [`MsgIdLine::expires`]。
    #[serde(default)]
    pub(crate) expires: Option<i64>,
}

/// `end` 行。
#[derive(Debug, Deserialize)]
struct EndLine {
    lines: u64,
    sha256: String,
}

/// 一行记录（内部 tag = `kind`；未知 `kind` 落到 [`Line::Unknown`]）。
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Line {
    Meta(MetaLine),
    Identity(IdentityLine),
    VuinCounter(CounterLine),
    MsgCounter(CounterLine),
    Config(ConfigLine),
    UserInfo(UserInfoLine),
    LastSeen(LastSeenLine),
    MsgId(MsgIdLine),
    Latest(LatestLine),
    End(EndLine),
    #[serde(other)]
    Unknown,
}

/// 校验通过、可直接落库的数据。
#[derive(Debug, Default)]
pub(crate) struct ImportData {
    /// `end` 之前的记录数（含未知 `kind`）。
    pub(crate) records: usize,
    /// 未知 `kind` 的行数。
    pub(crate) unknown_kinds: usize,
    /// `meta.allocator`（导出侧的 vUIN 分配方式）。
    pub(crate) allocator: Option<Allocator>,
    pub(crate) identities: Vec<IdentityLine>,
    pub(crate) vuin_counter: Option<i64>,
    pub(crate) msg_counter: Option<i64>,
    pub(crate) configs: Vec<ConfigLine>,
    pub(crate) user_infos: Vec<UserInfoRecord>,
    pub(crate) last_seen: Vec<LastSeenLine>,
    pub(crate) msg_ids: Vec<MsgIdLine>,
    pub(crate) latest: Vec<LatestLine>,
}

/// 解析并校验整份 JSONL。全部通过才返回，任何一条不合格都返回
/// [`ImportError::Format`]（含行号），调用方因此不会导入半个文件。
pub(crate) fn parse(bytes: &[u8]) -> Result<ImportData, ImportError> {
    let mut data = ImportData::default();
    let mut identities: HashMap<String, i64> = HashMap::new();
    let mut seen_meta = false;
    let mut end: Option<(usize, EndLine)> = None;
    let mut offset = 0usize;
    let mut number = 0usize;

    while offset < bytes.len() {
        let line_start = offset;
        let (raw_line, next) = split_line(bytes, line_start);
        number += 1;
        if end.is_some() {
            return Err(format_err(number, "`end` 行之后还有记录"));
        }
        // 行尾的 `\r`（Windows 编辑器回写）只影响解析，不参与 hash——hash 覆盖原始字节。
        let line = raw_line.strip_suffix(b"\r".as_slice()).unwrap_or(raw_line);
        if line.iter().all(u8::is_ascii_whitespace) {
            return Err(format_err(number, "空行"));
        }
        let record: Line = serde_json::from_slice(line)
            .map_err(|err| format_err(number, &format!("不是合法的 v1 记录: {err}")))?;
        offset = next;

        match record {
            Line::Meta(meta) => {
                if number != 1 {
                    return Err(format_err(number, "`meta` 必须是首行"));
                }
                if meta.format != FORMAT {
                    return Err(format_err(
                        number,
                        &format!("未知格式 `{}`（期望 `{FORMAT}`）", meta.format),
                    ));
                }
                if meta.version != FORMAT_VERSION {
                    return Err(format_err(
                        number,
                        &format!("不支持的版本 {}（期望 {FORMAT_VERSION}）", meta.version),
                    ));
                }
                seen_meta = true;
                data.allocator = meta.allocator;
                data.records += 1;
            }
            Line::End(end_line) => end = Some((line_start, end_line)),
            Line::Identity(line) => {
                check_positive(number, "vuin", line.vuin)?;
                check_text(number, "key", &line.key)?;
                check_text(number, "raw", &line.raw)?;
                if let Some(previous) = identities.insert(line.key.clone(), line.vuin) {
                    if previous != line.vuin {
                        return Err(format_err(
                            number,
                            &format!(
                                "身份 {} 重复出现且 vUIN 不一致（{previous} / {}）",
                                line.key, line.vuin
                            ),
                        ));
                    }
                    // 完全重复的行：忽略，但仍计入 `records`（与 `end.lines` 对齐）。
                } else {
                    data.identities.push(line);
                }
                data.records += 1;
            }
            Line::VuinCounter(line) => {
                check_non_negative(number, "counter", line.counter)?;
                if data.vuin_counter.replace(line.counter).is_some() {
                    return Err(format_err(number, "`vuin_counter` 出现了多次"));
                }
                data.records += 1;
            }
            Line::MsgCounter(line) => {
                check_non_negative(number, "counter", line.counter)?;
                if data.msg_counter.replace(line.counter).is_some() {
                    return Err(format_err(number, "`msg_counter` 出现了多次"));
                }
                data.records += 1;
            }
            Line::Config(line) => {
                check_text(number, "section", &line.section)?;
                check_text(number, "key", &line.key)?;
                data.configs.push(line);
                data.records += 1;
            }
            Line::UserInfo(line) => {
                check_text(number, "key", &line.key)?;
                // 库内 value 是 Go `json.Marshal(structs.FriendData)` 的字节；这里按同一形态
                // 紧凑序列化（`Value` 的对象按 key 排序，输出确定）。
                let json = serde_json::to_string(&line.value)
                    .map_err(|err| format_err(number, &format!("value 无法序列化: {err}")))?;
                data.user_infos.push(UserInfoRecord {
                    key: line.key,
                    json,
                });
                data.records += 1;
            }
            Line::LastSeen(line) => {
                check_positive(number, "vuin", line.vuin)?;
                check_text(number, "raw", &line.raw)?;
                data.last_seen.push(line);
                data.records += 1;
            }
            Line::MsgId(line) => {
                check_text(number, "msg_id", &line.msg_id)?;
                check_positive(number, "virtual", line.virtual_id)?;
                data.msg_ids.push(line);
                data.records += 1;
            }
            Line::Latest(line) => {
                check_text(number, "group", &line.group)?;
                check_text(number, "user", &line.user)?;
                check_text(number, "msg_id", &line.msg_id)?;
                data.latest.push(line);
                data.records += 1;
            }
            Line::Unknown => {
                data.unknown_kinds += 1;
                data.records += 1;
            }
        }
    }

    if !seen_meta {
        return Err(ImportError::Format("缺少 `meta` 行".to_owned()));
    }
    let Some((end_offset, end_line)) = end else {
        return Err(ImportError::Format("缺少 `end` 行".to_owned()));
    };
    let expected = u64::try_from(data.records).unwrap_or(u64::MAX);
    if end_line.lines != expected {
        return Err(ImportError::Format(format!(
            "`end.lines` 为 {}，实际记录数为 {expected}",
            end_line.lines
        )));
    }
    let actual = sha256_hex(&bytes[..end_offset]);
    let declared = end_line.sha256.to_ascii_lowercase();
    if actual != declared {
        return Err(ImportError::Format(format!(
            "`end.sha256` 不匹配：文件校验 {actual}，声明 {declared}"
        )));
    }
    Ok(data)
}

/// 取一行及其下一行起点（不含行尾 `\n`）。
fn split_line(bytes: &[u8], offset: usize) -> (&[u8], usize) {
    match bytes[offset..].iter().position(|&byte| byte == b'\n') {
        Some(position) => (&bytes[offset..offset + position], offset + position + 1),
        None => (&bytes[offset..], bytes.len()),
    }
}

/// `end` 行之前所有原始字节的 SHA-256 小写十六进制。
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn format_err(number: usize, detail: &str) -> ImportError {
    ImportError::Format(format!("第 {number} 行: {detail}"))
}

fn check_text(number: usize, field: &str, value: &str) -> Result<(), ImportError> {
    if value.is_empty() {
        return Err(format_err(number, &format!("`{field}` 不能为空")));
    }
    Ok(())
}

fn check_positive(number: usize, field: &str, value: i64) -> Result<(), ImportError> {
    if value <= 0 {
        return Err(format_err(
            number,
            &format!("`{field}` 必须是正整数，实际 {value}"),
        ));
    }
    Ok(())
}

fn check_non_negative(number: usize, field: &str, value: i64) -> Result<(), ImportError> {
    if value < 0 {
        return Err(format_err(
            number,
            &format!("`{field}` 不能为负，实际 {value}"),
        ));
    }
    Ok(())
}
