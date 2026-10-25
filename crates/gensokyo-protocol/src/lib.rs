//! gensokyo-protocol：OneBot V11 模型、CQ 码、OutboundPlan、handler 注册表。
//!
//! 该 crate 永不依赖 `qq` / `storage` / `media`，保证协议层可纯 JSON 单测。
//!
//! # M1f 覆盖范围
//! - [`event`]：`post_type` 全形态（`message` / `message_sent` / `notice` / `request` /
//!   `meta_event`）、消息段两种 `message` 形态、`sender` / `anonymous` / `file` 嵌套类型、
//!   通知分类 [`event::NoticeKind`]。
//! - [`segment`]：消息段模型与 CQ / 数组双向转换、[`segment::MediaRef`]。
//! - [`cqcode`]：CQ 转义/反转义、分词解析与生成、码表 [`cqcode::CODES`]（at / reply / image /
//!   record / video / file / face / markdown / card / keyboard / stream / input_notify /
//!   music（qqmusic）/ active / wakeup / node / set_group / remove / poke 等）。
//! - [`action`]：`ActionRequest` / `ActionResponse` 与常用 action 的参数、结果类型。
//! - [`outbound`]：`foundItems` 的类型化替代 [`outbound::OutboundPlan`]，含
//!   `from_found_items` / `to_found_items` / `from_segments` / `to_segments`；markdown /
//!   keyboard / card / stream / input_notify 的载荷类型与「与 Go 的字段差异」见模块文档。
//! - [`registry`]：`Handler` trait + 显式注册表、别名与 [`registry::KNOWN_ACTIONS`]。
//!   handler 返回 [`registry::HandlerFuture`]，runtime 直接 await（无 `spawn_blocking`）。
//!
//! # 与 Go 的口径差异
//! - **出站转义只转 `,` `]` `&`**（[`cqcode::escape`]），不转 `[`——跟随 `handlers/cqcode.go`。
//!   `internal/domain/message/cq.go` 的 `EscapeCQ` 会转 `[`，两处原本就不一致；协议层对外
//!   只保留 M0 钉死的 Go 出站口径。
//! - **CQ 解析以第一个 `]` 结束**（对齐 `internal/domain/message/parse_string.go`）。
//!   `handlers/cqcode.go` 的 markdown 正则用贪婪 `\{.*\}`，能容忍载荷里未转义的 `]`；
//!   本实现要求生成侧先把 `]` 转成 `&#93;`，因此往返仍无损。
//! - **裸 `,` 是参数分隔符**：`[CQ:file,file=a,b.txt]` 按 Go 语义只取 `file=a`，`b.txt`
//!   因既无 `=` 也无 `:` 被丢弃；需要逗号时用 `&#44;`。无 `=` 的片段按首个 `:` 再切一次
//!   （`[CQ:stream,type:start,qq:123]` 是 Go `streamPattern` 的原生写法）。
//! - **card / keyboard / markdown 载荷按 `foundItems` 契约归一**：`markdown` 是 base64(JSON)，
//!   `keyboard` 是 JSON 串，`card` 是 JSON 对象串——`[CQ:card,title=…,desc=…]` 的 `k=v`
//!   会聚合成对象（口径见 [`outbound::OutboundPlan::from_segments`]）。
//! - 事件里的 `sub_type` / `notice_type` / `real_*` 保持字符串（QQ 仍在扩展），未知字段落进
//!   各事件的 `extra`，不会导致反序列化失败。
//! - `OutboundPlan.unknown_images` / `unknown_records` 与 Go 的 `unknown_image` /
//!   `unknown_record` 一一对应：Go 的图片与语音兜底发送是两条不同路径（`handlers/`
//!   分别走图片/语音通道），模型层不再合并成一个桶。
//!
//! # 不在本 crate
//! QQ OpenAPI 调用、runtime、action 的具体业务逻辑（M4 `gensokyo-actions`）。

use gensokyo_core::Ob11Id;

mod base64;

pub mod action;
pub mod cqcode;
pub mod event;
pub mod outbound;
pub mod registry;
pub mod segment;

pub use action::{ActionRequest, ActionResponse, Echo, Status};
pub use event::Event;
pub use outbound::OutboundPlan;
pub use registry::{Handler, HandlerFuture, Registry};
pub use segment::{MediaRef, MessageContent, Segment};

/// JSON 附加字段：`#[serde(flatten)]` 的落点，未知字段不会丢失。
pub type Extra = serde_json::Map<String, serde_json::Value>;

/// 把 OneBot 线上 ID 文本还原为 [`Ob11Id`]。
///
/// 纯十进制数字（可带负号）走 [`Ob11Id::Num`]，其余（32 位 OpenID、flag 型 ID）保持字符串
/// 形态。Go 侧对应 `strconv.ParseInt` 后回写，语义一致。
#[must_use]
pub fn parse_id(raw: &str) -> Ob11Id {
    match raw.parse::<i64>() {
        Ok(value) => Ob11Id::Num(value),
        Err(_) => Ob11Id::Str(raw.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Ob11Id, parse_id};

    #[test]
    fn numeric_text_becomes_num() {
        assert_eq!(parse_id("123456"), Ob11Id::Num(123_456));
        assert_eq!(parse_id("-1"), Ob11Id::Num(-1));
    }

    #[test]
    fn openid_text_stays_string() {
        assert_eq!(
            parse_id("BAC3N5VPRKPRQ1GJ"),
            Ob11Id::Str("BAC3N5VPRKPRQ1GJ".to_owned())
        );
        assert_eq!(parse_id(""), Ob11Id::Str(String::new()));
    }
}
