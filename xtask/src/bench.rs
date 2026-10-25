//! `xtask bench`：固定迭代的同步微基准（不引入 criterion 等第三方框架）。
//!
//! 这里的数字只回答"某条路径是不是慢了一个量级"，不做统计显著性。方法论、复跑方式与
//! 历史基线见 `docs/rust-perf.md`；bench 只覆盖可在单进程内同步跑完的热路径，
//! 需要 tokio / 网络 / 磁盘的链路不在范围内。

use std::hint::black_box;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use gensokyo_protocol::Event;
use serde_json::Value;

/// 默认正式迭代次数（`--iters` 可改）。
pub const DEFAULT_ITERS: u64 = 100_000;

/// 正式计时前的预热次数：摊掉首次分配、惰性初始化与分支预测冷启动。
const WARMUP_ITERS: u64 = 1_000;

/// 一条基准结果。
struct BenchResult {
    name: &'static str,
    iters: u64,
    elapsed: Duration,
}

impl BenchResult {
    /// 单次操作耗时（纳秒）。
    fn ns_per_op(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1e9 / self.iters as f64
    }

    /// 总耗时（毫秒）。
    fn total_ms(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1e3
    }
}

/// 跑全部用例并打印表格。
///
/// # Errors
/// `--iters 0`、样本 JSON 无法解析/规划时返回错误：样本坏了必须在计时前炸掉，
/// 否则表格里会出现"很快但什么都没测"的假基线。
pub fn run(iters: u64) -> Result<()> {
    if iters == 0 {
        bail!("--iters 必须大于 0");
    }

    let results = [
        bench_cq_parse(iters),
        bench_event_decode(iters)?,
        bench_plan_send_group_msg(iters)?,
    ];

    // 输出保持 ASCII：基线要能跨控制台代码页原样粘贴进文档与 PR。
    println!(
        "platform: {}-{} | profile: release (lto=thin, codegen-units=1, opt-level=3)",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("warmup: {WARMUP_ITERS} iters/case | measured: {iters} iters/case");
    println!();
    println!(
        "{:<34} {:>10} {:>12} {:>12}",
        "name", "iters", "total_ms", "ns/op"
    );
    for result in &results {
        println!(
            "{:<34} {:>10} {:>12.3} {:>12.1}",
            result.name,
            result.iters,
            result.total_ms(),
            result.ns_per_op()
        );
    }
    Ok(())
}

/// 预热 + 固定迭代计时；`black_box` 防止被测结果被优化掉。
///
/// 闭包返回一个廉价哨兵值（长度 / 布尔），计时区间内不做任何额外分配。
fn measure<R>(name: &'static str, iters: u64, mut op: impl FnMut() -> R) -> BenchResult {
    for _ in 0..WARMUP_ITERS {
        black_box(op());
    }
    let start = Instant::now();
    for _ in 0..iters {
        black_box(op());
    }
    BenchResult {
        name,
        iters,
        elapsed: start.elapsed(),
    }
}

/// 20 段混合 CQ 文本：文本与 at / image / face / reply / record 交替，贴近群聊真实负载。
fn cq_sample() -> String {
    const CODES: [&str; 5] = [
        "[CQ:at,qq=10001]",
        "[CQ:image,file=8f3c.png,url=https://multimedia.nt.qq.com.cn/download?appid=1407]",
        "[CQ:face,id=14]",
        "[CQ:reply,id=1234567890123]",
        "[CQ:record,file=abc.silk]",
    ];
    let mut text = String::new();
    for index in 0..20 {
        text.push('第');
        text.push_str(&index.to_string());
        text.push_str("段 ");
        text.push_str(CODES[index % CODES.len()]);
    }
    text
}

/// 一条典型 `GROUP_MESSAGE_CREATE`：数组形态消息、sender 块、未知字段（落 `extra`）。
const GROUP_MESSAGE_EVENT: &str = r#"{
  "post_type": "message",
  "message_type": "group",
  "sub_type": "normal",
  "time": 1700000000,
  "self_id": 10000,
  "message_id": 8888,
  "user_id": 12345,
  "group_id": 99999,
  "raw_message": "求闻史纪 [CQ:at,qq=10001] 第七版",
  "message": [
    {"type": "text", "data": {"text": "求闻史纪 "}},
    {"type": "at", "data": {"qq": "10001"}},
    {"type": "text", "data": {"text": " 第七版"}},
    {"type": "image", "data": {"file": "8f3c.png", "url": "https://multimedia.nt.qq.com.cn/download?appid=1407"}}
  ],
  "font": 0,
  "sender": {"user_id": 12345, "nickname": "稗田阿求", "card": "阿求", "role": "member", "sex": "unknown", "age": 0},
  "real_user_id": "A1B2C3D4E5F6",
  "to_me": true,
  "extra_qq_field": {"nested": [1, 2, 3]}
}"#;

/// 一条 5 段 `send_group_msg` 参数（文本 / at / 图片 / 表情 / 文本）。
const SEND_GROUP_PARAMS: &str = r#"{
  "group_id": 99999,
  "auto_escape": false,
  "message": [
    {"type": "text", "data": {"text": "求闻史纪 "}},
    {"type": "at", "data": {"qq": "10001"}},
    {"type": "image", "data": {"file": "8f3c.png", "url": "https://multimedia.nt.qq.com.cn/download?appid=1407"}},
    {"type": "face", "data": {"id": "14"}},
    {"type": "text", "data": {"text": " 第七版"}}
  ]
}"#;

/// a) CQ 解析：入站消息文本 → 消息段。
fn bench_cq_parse(iters: u64) -> BenchResult {
    let text = cq_sample();
    measure("cqcode::parse 20-segment mixed", iters, || {
        gensokyo_protocol::cqcode::parse(black_box(text.as_str())).len()
    })
}

/// b) 事件反序列化：GROUP_MESSAGE_CREATE JSON → `Event`。
fn bench_event_decode(iters: u64) -> Result<BenchResult> {
    serde_json::from_str::<Event>(GROUP_MESSAGE_EVENT)
        .context("GROUP_MESSAGE_CREATE 样本反序列化失败")?;
    Ok(measure("Event::deserialize group message", iters, || {
        serde_json::from_str::<Event>(black_box(GROUP_MESSAGE_EVENT)).is_ok()
    }))
}

/// c) 出站规划：`send_group_msg` 参数 → `SendRequest`。
fn bench_plan_send_group_msg(iters: u64) -> Result<BenchResult> {
    let params: Value =
        serde_json::from_str(SEND_GROUP_PARAMS).context("send_group_msg 样本不是合法 JSON")?;
    gensokyo_actions::plan_send_group_msg(&params)
        .map_err(|err| anyhow::anyhow!("plan_send_group_msg 样本规划失败: {err}"))?;
    Ok(measure("plan_send_group_msg 5-segment", iters, || {
        gensokyo_actions::plan_send_group_msg(black_box(&params))
            .expect("plan_send_group_msg 在基准样本上失败")
            .message
            .text
            .len()
    }))
}
