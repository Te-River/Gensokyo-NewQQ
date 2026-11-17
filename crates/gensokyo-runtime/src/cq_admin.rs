//! 出站管理 CQ 码的运行期执行（P83）：[`OutboundPlan::admin_ops`] → QQ 上游。
//!
//! Go 的 `ProcessOutboundCQCodes`（`handlers/cqcode.go:226-244`）在 `send_group_msg` 里
//! 左到右单趟执行 `[CQ:member]` / `[CQ:set_group]` / `[CQ:remove]`；这里挂进
//! [`crate::deferred::execute_inner`] 的**判空之前**（纯动作消息执行动作后按空消息回执，
//! 设计 §3.3-1），并以 `depth == 0` 守门防止 URL 变换的 QR 子请求双执行（§3.3-2）。
//!
//! # 与 Go 的刻意差异（parity delta，lead 裁决 2026-11-15）
//!
//! - **失败大声拒绝**：Go 对反查失败 / 上游失败一律吞码只留日志；这里按全 crate 口径
//!   `Protocol` → `1400`、`Upstream` / 其余 → `1500`，文案含 action 与具体 ID。
//! - **多码逐条尝试 + 聚合**：对齐 Go「任一失败不中断」，全部跑完后以**第一个错误**作为
//!   整体结果；先执行过的动作不回滚，每条成败各自 WARN。
//! - **私聊 remove 走 C2C 撤回**（`RetractRequest.user_openid`）：Go 硬编码群撤回，在私聊
//!   里是缺陷，这里修复（登记为「修复 Go 缺陷」型差异）。撤回目标恒为**发送目标**
//!   （群聊 = 目标群，私聊 = 目标用户），`[CQ:remove]` 的 `group_id` 参数一律不读——
//!   否则一条消息可跨群撤回（安全约束 m1/m5）。
//! - **私聊 member / set_group 留字面**：Go 私聊路径不消费管理码
//!   （`ProcessOutboundCQCodes` 只在 `send_group_msg.go:198/:544` 调用），同样不静默丢弃。
//! - **`cq_admin_actions=off`**：消息含**任何**管理码即整条 `1400` 大声拒绝（逃生阀；
//!   默认 `legacy` 照 Go 执行）。闸门在私聊分流 / 逐条执行之前，零上游调用。
//! - **`whole_ban` 只告警、不执行**：官方 v2 无全员禁言写入字段，对齐 Go `cqcode.go:484-508`
//!   「仅日志 + `return ""`」——记 WARN 后吞掉该码，正文照发；不再让整条消息 `1500` 丢正文。
//!
//! # 审计（lead 裁决 8）
//!
//! 每个码执行前打一条 WARN：`action` / `group` / `users` / `raw_cq` / `target`——
//! drive-by（管理码藏在正文中随转发 / 复读流动）可在日志里定位到是哪条内容触发的。

use gensokyo_actions::{ActionContext, RetractRequest, SendRequest, SendTarget};
use gensokyo_core::{Error, Result};
use gensokyo_protocol::outbound::{AdminOp, MemberDirective, MemberKind, RemoveOp, SetGroupOp};
use gensokyo_protocol::parse_id;
use gensokyo_qq::dto::{ApprovalJoinRequest, MemberBlacklistRequest};
use gensokyo_storage::echo::echo_key_event_id_v2;

use crate::deferred::{DeferredDeps, resolve_reply};

/// [`run`] 的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminOutcome {
    /// `Some(Some(id))` = `[CQ:member,type=add]` 命中入群事件（本条消息以它被动回复）；
    /// `Some(None)` = `type=remove` 显式清空（转主动推送）；`None` = 未命中 / 无 member 码。
    pub passive_event_id: Option<Option<String>>,
}

/// [`crate::deferred::execute_inner`] 管理码阶段的产物。
#[derive(Debug, Default)]
pub(crate) struct AdminStage {
    /// 私聊留字面改写后的计划副本（`None` = 无需改写）。
    pub(crate) request: Option<SendRequest>,
    /// member 的被动身份覆盖，透传给 [`crate::deferred`] 在 resolve_passive 之后应用。
    pub(crate) passive_event_id: Option<Option<String>>,
}

/// 管理码阶段：depth 守门 + 私聊分流 + 执行 + 私聊留字面。
///
/// `openid` 是**发送目标**的 OpenID（群聊 = 群，私聊 = 用户）。返回值见 [`AdminStage`]。
pub(crate) async fn stage(
    context: &ActionContext,
    plan: &SendRequest,
    deps: &DeferredDeps<'_>,
    openid: &str,
    depth: usize,
) -> Result<AdminStage> {
    let ops = &plan.message.plan.admin_ops;
    if depth > 0 || ops.is_empty() {
        return Ok(AdminStage::default());
    }
    // 逃生阀（lead 裁决 1 / F4）：off 下消息含**任何**管理码即整条拒绝——闸门必须在
    // 私聊分流与逐条执行之前，否则私聊 member / set_group 会把字面照发出去。零上游调用。
    if context.config().snapshot().get_cq_admin_actions() == "off" {
        return Err(Error::Protocol(format!(
            "cq_admin_actions=off：已拒绝执行 {}",
            ops.first().map(AdminOp::to_cq_literal).unwrap_or_default()
        )));
    }
    match plan.target {
        // 群聊：三码全量执行（Go 只在群发路径调 ProcessOutboundCQCodes）。
        SendTarget::Group(_) => {
            let outcome = run(context, &plan.target, openid, ops, deps).await?;
            Ok(AdminStage {
                request: None,
                passive_event_id: outcome.passive_event_id,
            })
        }
        // 私聊：member / set_group 不执行、按规范化字面回写正文（Go 同，不静默丢弃）；
        // remove 走 C2C 撤回（修复 Go 硬编码群撤回的缺陷）。
        SendTarget::Private(_) => {
            let runnable: Vec<AdminOp> = ops
                .iter()
                .filter(|op| matches!(op, AdminOp::Remove(_)))
                .cloned()
                .collect();
            let outcome = if runnable.is_empty() {
                AdminOutcome::default()
            } else {
                run(context, &plan.target, openid, &runnable, deps).await?
            };
            let mut literals = String::new();
            for op in ops.iter().filter(|op| !matches!(op, AdminOp::Remove(_))) {
                tracing::warn!(
                    action = %op.action_name(),
                    group = %audit_group(op, openid),
                    users = %audit_users(op),
                    target = ?plan.target,
                    raw_cq = %op.to_cq_literal(),
                    "[CQ 管理码] 私聊不执行，留在正文"
                );
                literals.push_str(&op.to_cq_literal());
            }
            let request = (!literals.is_empty()).then(|| {
                let mut rewritten = plan.clone();
                rewritten.message.text.push_str(&literals);
                rewritten.message.plan.admin_ops.clear();
                rewritten
            });
            Ok(AdminStage {
                request,
                passive_event_id: outcome.passive_event_id,
            })
        }
    }
}

/// 执行一批管理码（按到达顺序串行，对齐 Go 单趟 `ReplaceAllStringFunc`，任一失败不中断）。
///
/// 全部跑完后以**第一个错误**作为整体结果（`Protocol` → `1400` / `Upstream` 与其余 →
/// `1500`），每条成败各自 WARN。`openid` 是发送目标的 OpenID（群聊 = 群，私聊 = 用户）。
///
/// # Errors
/// L2 数值非法（duration / enable / approve）、ID 反查失败（[`Error::Protocol`]），
/// 或上游 API 失败（[`Error::Upstream`] 等）。`cq_admin_actions=off` 的整条拒绝在
/// [`stage`] 前置完成，不会走到这里。
pub(crate) async fn run(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    ops: &[AdminOp],
    deps: &DeferredDeps<'_>,
) -> Result<AdminOutcome> {
    let mut outcome = AdminOutcome::default();
    let mut first_error = None;
    for op in ops {
        // 审计（lead 裁决 8）：执行前先落一条 WARN，drive-by 可从日志定位来源。
        tracing::warn!(
            action = %op.action_name(),
            group = %audit_group(op, openid),
            users = %audit_users(op),
            raw_cq = %op.to_cq_literal(),
            target = ?target,
            "[CQ 管理码] 执行出站管理码"
        );
        match execute_op(context, target, openid, op, deps, &mut outcome).await {
            Ok(()) => {}
            Err(err) => {
                tracing::warn!(
                    action = %op.action_name(),
                    group = %audit_group(op, openid),
                    users = %audit_users(op),
                    error = %err,
                    "[CQ 管理码] 单条执行失败，继续尝试后续"
                );
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }
    match first_error {
        Some(err) => Err(err),
        None => Ok(outcome),
    }
}

/// 单个码的执行。member 只写 `outcome.passive_event_id`，零 API 调用。
async fn execute_op(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    op: &AdminOp,
    deps: &DeferredDeps<'_>,
    outcome: &mut AdminOutcome,
) -> Result<()> {
    match op {
        AdminOp::Member(directive) => {
            // 零 API 调用（Go cqcode.go:313-331）：add 从 echo 取入群 event_id，remove
            // 显式清空。反查失败按 Go 口径只告警、不改身份（cqcode.go:304-310 回退原值
            // 后查键必然未命中，等价于不覆盖）。
            let event = member_event_id(context, deps, openid, directive);
            match (directive.kind, event) {
                (MemberKind::Add, Some(event_id)) => {
                    outcome.passive_event_id = Some(Some(event_id));
                }
                (MemberKind::Add, None) => {
                    tracing::warn!(
                        group = %audit_group(op, openid),
                        "[CQ:member] 入群回复：未找到 event_id，不改发送身份"
                    );
                }
                (MemberKind::Remove, _) => {
                    outcome.passive_event_id = Some(None);
                }
            }
            Ok(())
        }
        AdminOp::Remove(op) => execute_remove(context, target, openid, op, deps).await,
        AdminOp::SetGroup(set_group) => {
            let raw_cq = op.to_cq_literal();
            execute_set_group(context, openid, set_group, &raw_cq).await
        }
    }
}

/// `[CQ:member,type=add]` 的 echo 查键（Go `appID + "_" + groupOpenID`，cqcode.go:315-317）。
fn member_event_id(
    context: &ActionContext,
    deps: &DeferredDeps<'_>,
    default_group: &str,
    directive: &MemberDirective,
) -> Option<String> {
    let echo = deps.echo?;
    let group = match directive.group_id.as_deref() {
        Some(raw) => match context.resolve_openid("group_id", raw, "group_openid") {
            Ok(openid) => openid,
            Err(err) => {
                tracing::warn!(group_id = %raw, error = %err, "[CQ:member] 群 OpenID 反查失败");
                return None;
            }
        },
        None => default_group.to_owned(),
    };
    let app_id = context.config().snapshot().get_app_id().to_string();
    let stored = echo.get_event_id_by_key(&echo_key_event_id_v2(&app_id, &group));
    (!stored.is_empty()).then_some(stored)
}

/// `[CQ:remove]`：撤回。目标恒为**发送目标**——群聊走群撤回（目标群），私聊走 C2C
/// （`RetractRequest.user_openid`，lead 裁决 6）。`op.group_id` 一律不读：读它会让一条群消息
/// 撤回另一个群的消息（m1/m5）。缺 `msg_id` 时仅群聊自动取该用户最新消息。
async fn execute_remove(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    op: &RemoveOp,
    deps: &DeferredDeps<'_>,
) -> Result<()> {
    // user_id 仅用于反查与缺 msg_id 时定位消息（Go cqcode.go:376-380）。
    let user_id = op
        .user_id
        .as_deref()
        .ok_or_else(|| Error::Protocol("[CQ:remove] 缺少 user_id".to_owned()))?;
    let member = context.resolve_openid("user_id", user_id, "member_openid")?;
    let message_id = match op.msg_id.as_deref() {
        Some(msg_id) => resolve_reply(context, deps.echo, &parse_id(msg_id))?,
        None => match target {
            SendTarget::Group(_) => latest_message_id(context, openid, &member)?,
            // idmap 只按「群 + 成员」记录最新消息，C2C 没有对应键：私聊必须显式给 msg_id
            SendTarget::Private(_) => {
                return Err(Error::Protocol(
                    "[CQ:remove] 私聊撤回必须带 msg_id".to_owned(),
                ));
            }
        },
    };
    let request = match target {
        SendTarget::Group(_) => RetractRequest {
            group_openid: Some(openid.to_owned()),
            user_openid: None,
            message_id,
        },
        SendTarget::Private(_) => RetractRequest {
            group_openid: None,
            user_openid: Some(openid.to_owned()),
            message_id,
        },
    };
    context.api().delete_msg(request).await
}

/// 缺 `msg_id` 时自动取该用户最新消息（Go `idmap.GetLatestMsgID`，cqcode.go:350-369）。
///
/// Go 在这里留正文（`return match`）；按 L2 口径（lead 裁决 2）改为大声拒绝并说明原因。
fn latest_message_id(context: &ActionContext, group: &str, member: &str) -> Result<String> {
    let Some(idmap) = context.idmap() else {
        return Err(Error::Protocol(
            "[CQ:remove] 缺 msg_id 且未装配 idmap，无法自动取该用户最新消息".to_owned(),
        ));
    };
    match idmap.get_latest_msg_id(group, member) {
        Ok(latest) if !latest.is_empty() => Ok(latest),
        // 未记录 / 已过期（is_not_found 覆盖两者）都算「取不到」；其余存储错误走 1500
        Err(err) if !err.is_not_found() => {
            Err(Error::Storage(format!("自动获取用户最新消息失败: {err}")))
        }
        Ok(_) | Err(_) => Err(Error::Protocol(format!(
            "[CQ:remove] 群 {group} 用户 {member} 没有可撤回的最新消息，请带 msg_id 重试"
        ))),
    }
}

/// `set_group` 系（7 个 action）。`default_group` 是发送目标群 OpenID；`raw_cq` 仅用于审计。
async fn execute_set_group(
    context: &ActionContext,
    default_group: &str,
    op: &SetGroupOp,
    raw_cq: &str,
) -> Result<()> {
    match op {
        SetGroupOp::Ban {
            group_id,
            user_id,
            duration,
            raw_duration,
        } => {
            // 数值非法归 L2（设计 §5）：Go 对非数字静默当 0，这里大声拒绝。
            if let Some(raw) = raw_duration {
                return Err(Error::Protocol(format!(
                    "[CQ:set_group] ban: duration 非法: {raw}"
                )));
            }
            let group = resolve_group(context, group_id.as_deref(), default_group)?;
            let member = context.resolve_openid("user_id", user_id, "member_openid")?;
            context
                .api()
                .set_group_ban(&group, &member, *duration)
                .await
        }
        SetGroupOp::WholeBan {
            group_id,
            raw_enable,
            ..
        } => {
            // 官方 v2 无全员禁言写入字段（`restrict_chat_setting` 只有 members）：对齐 Go
            // `cqcode.go:484-508`「仅日志 + return ""」——吞掉该码、正文照发、零上游调用；
            // 不再整条 1500 丢正文（F2）。`enable` 非法同样只告警（Go 也不执行）。
            tracing::warn!(
                action = "set_group:whole_ban",
                group = %group_id.as_deref().unwrap_or(default_group),
                raw_cq = %raw_cq,
                raw_enable = ?raw_enable,
                "[CQ 管理码] whole_ban 官方无写入字段，仅告警并跳过"
            );
            Ok(())
        }
        SetGroupOp::AddRequest {
            group_id,
            user_id,
            flag,
            approve,
            raw_approve,
            reason,
            add_to_blacklist,
        } => {
            if let Some(raw) = raw_approve {
                return Err(Error::Protocol(format!(
                    "[CQ:set_group] add_request: approve 非法: {raw}"
                )));
            }
            let group = resolve_group(context, group_id.as_deref(), default_group)?;
            let member = context.resolve_openid("user_id", user_id, "member_openid")?;
            let request = ApprovalJoinRequest {
                op: if *approve { "approve" } else { "decline" }.to_owned(),
                join_request_id: flag.clone(),
                reject_reason: reason.clone(),
                add_to_member_blacklist: add_to_blacklist.unwrap_or(false),
            };
            context
                .api()
                .approve_join_request(&group, &member, &request)
                .await
        }
        SetGroupOp::Strategy {
            execute,
            strategy_id,
        } => {
            if *execute {
                context
                    .api()
                    .execute_join_approval_strategy(strategy_id)
                    .await
            } else {
                context
                    .api()
                    .delete_join_approval_strategy(strategy_id)
                    .await
            }
        }
        SetGroupOp::Kick {
            group_id,
            user_ids,
            add_blacklist,
        } => {
            let group = resolve_group(context, group_id.as_deref(), default_group)?;
            // 部分反查失败跳过不中断（Go cqcode.go:696-698），全失败才大声拒绝。
            let openids = resolve_members(context, "kick", user_ids)?;
            let result = context
                .api()
                .set_group_kick(&group, &openids, *add_blacklist)
                .await?;
            if !result.add_to_member_blacklist_fail_openids.is_empty() {
                tracing::warn!(
                    group = %group,
                    failed = ?result.add_to_member_blacklist_fail_openids,
                    "[CQ:set_group] kick: 部分成员拉黑失败"
                );
            }
            Ok(())
        }
        SetGroupOp::Blacklist {
            add,
            group_id,
            user_ids,
        } => {
            let group = resolve_group(context, group_id.as_deref(), default_group)?;
            let openids = resolve_members(context, "blacklist", user_ids)?;
            let request = MemberBlacklistRequest {
                op: if *add { "add" } else { "del" }.to_owned(),
                member_openids: openids,
            };
            let response = context
                .api()
                .update_member_blacklist(&group, &request)
                .await?;
            if !response.fail_openids.is_empty() {
                tracing::warn!(
                    group = %group,
                    failed = ?response.fail_openids,
                    "[CQ:set_group] blacklist: 部分成员操作失败"
                );
            }
            Ok(())
        }
    }
}

/// `group_id` 缺省用发送目标群；显式给值时反查（Go 吞码，这里大声拒绝）。
fn resolve_group(
    context: &ActionContext,
    group_id: Option<&str>,
    default_group: &str,
) -> Result<String> {
    match group_id {
        Some(raw) => context.resolve_openid("group_id", raw, "group_openid"),
        None => Ok(default_group.to_owned()),
    }
}

/// 逐个反查成员 OpenID（32 位直通）：失败的跳过并告警、不中断整批（Go cqcode.go:696-698）；
/// **全部**失败才大声拒绝（避免静默少踢）。
fn resolve_members(context: &ActionContext, action: &str, ids: &[String]) -> Result<Vec<String>> {
    let mut openids = Vec::with_capacity(ids.len());
    for id in ids {
        match context.resolve_openid("user_id", id, "member_openid") {
            Ok(openid) => openids.push(openid),
            Err(err) => {
                tracing::warn!(
                    action = %action,
                    user_id = %id,
                    error = %err,
                    "[CQ:set_group] 成员反查失败，已跳过"
                );
            }
        }
    }
    if openids.is_empty() {
        return Err(Error::Protocol(format!(
            "[CQ:set_group] {action}: user_id 反查全部失败: {}",
            ids.join(",")
        )));
    }
    Ok(openids)
}

/// 审计的 `group` 字段：op 显式 `group_id` 优先，缺省记发送目标。
///
/// `remove` 例外：执行期不读 `group_id`（撤回目标恒为发送目标，m1/m5），审计同样以发送
/// 目标为准，避免日志指向一个不会被撤回的群（原值仍在 `raw_cq` 里）。
fn audit_group(op: &AdminOp, openid: &str) -> String {
    let explicit = match op {
        AdminOp::Member(directive) => directive.group_id.as_deref(),
        AdminOp::Remove(_) => None,
        AdminOp::SetGroup(
            SetGroupOp::Ban { group_id, .. }
            | SetGroupOp::WholeBan { group_id, .. }
            | SetGroupOp::AddRequest { group_id, .. }
            | SetGroupOp::Kick { group_id, .. }
            | SetGroupOp::Blacklist { group_id, .. },
        ) => group_id.as_deref(),
        AdminOp::SetGroup(SetGroupOp::Strategy { .. }) => None,
    };
    explicit.unwrap_or(openid).to_owned()
}

/// 审计的 `users` 字段：单用户的原值 / 批量的合并列表。
fn audit_users(op: &AdminOp) -> String {
    match op {
        AdminOp::Member(directive) => directive.user_id.clone().unwrap_or_default(),
        AdminOp::Remove(op) => op.user_id.clone().unwrap_or_default(),
        AdminOp::SetGroup(SetGroupOp::Ban { user_id, .. }) => user_id.clone(),
        AdminOp::SetGroup(SetGroupOp::AddRequest { user_id, .. }) => user_id.clone(),
        AdminOp::SetGroup(SetGroupOp::Kick { user_ids, .. })
        | AdminOp::SetGroup(SetGroupOp::Blacklist { user_ids, .. }) => user_ids.join(","),
        AdminOp::SetGroup(_) => String::new(),
    }
}
