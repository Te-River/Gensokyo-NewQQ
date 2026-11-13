//! xurls v1.1.0 Go 金样逐 span 断言（P56-T1，设计 §5.1）。
//!
//! 金样由一次性 Go harness（同版本 `mvdan.cc/xurls v1.1.0`，
//! `Relaxed.FindAllStringIndex`，已用后即删）对 155 条 corpus 导出：
//! 中文全角括号、尾随标点、TLD 表内/外、IPv4/IPv6、otherScheme、
//! 嵌套括号、scheme 大小写、CQ 残留、纯数字、端口、midChar 路径字符等。
//! 偏移为字节偏移，与 [`gensokyo_protocol::xurls::UrlSpan`] 同口径。

use gensokyo_protocol::xurls::{UrlSpan, find_urls};
use serde_json::Value;

#[test]
fn relaxed_golden_spans() {
    let raw = include_str!("xurls_golden.json");
    let cases: Vec<Value> = serde_json::from_str(raw).expect("金样 JSON 解析失败");
    assert!(cases.len() >= 120, "corpus 规模不足: {}", cases.len());
    for (i, case) in cases.iter().enumerate() {
        let text = case["text"].as_str().expect("text 字段缺失");
        let expected: Vec<UrlSpan> = case["spans"]
            .as_array()
            .expect("spans 字段缺失")
            .iter()
            .map(|s| UrlSpan {
                start: s[0].as_u64().expect("span start 非整数") as usize,
                end: s[1].as_u64().expect("span end 非整数") as usize,
            })
            .collect();
        let actual = find_urls(text);
        assert_eq!(actual, expected, "金样 case #{i} text={text:?}");
    }
}