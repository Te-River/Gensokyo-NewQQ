package cqparse

import (
	"context"
	"errors"

	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// Deps 是 Phase2 执行器的注入接口（架构设计 §3）。
// cqparse 零 botgo/openapi import：API 依赖全部经本接口注入，
// 实现留在 handlers 包（cqparse_exec.go），apiv2 经闭包携带。

// GroupInfoData 是群信息取数的最小投影（与 get_group_info.go 同源字段）。
type GroupInfoData struct {
	Name        string
	Memo        string
	MemberCount int
}

// GroupInfoFetcher 抽象 apiv2.GroupInfo 只读取数。
type GroupInfoFetcher interface {
	GroupInfo(ctx context.Context, groupOpenID string) (*GroupInfoData, error)
}

// Deps 汇集全部外部依赖；任一字段可为 nil（对应 handler 走失败回退）。
type Deps struct {
	GroupInfo GroupInfoFetcher
	// AvatarURL(qq, groupID, hasGroup) 返回头像 URL；反查失败返回 error。
	AvatarURL func(qq, groupID string, hasGroup bool) (string, error)
	// RunAction 执行动作码（member/remove/set_group）；eventID 由调用方持有，
	// member add/remove 的回填语义与今日 ProcessOutboundCQCodes 逐字节一致。
	RunAction func(p PendingAction, eventID *string) ExecOutcome
}

// PendingAction 是动作码的待执行描述；Parse 只产出不执行。
type PendingAction struct {
	Action string
	Params map[string]string
	Raw    string
	Scope  Scope
	// DefaultGroupID 为当前会话虚拟群 ID，供执行器对缺省 group_id 回退（Q1）。
	DefaultGroupID string
}

// ExecOutcome 是动作执行产物：member 跨群路由 realGroupID、回填 eventID。
type ExecOutcome struct {
	RealGroupID string
	EventID     string
	Intercepted bool // 私聊/转发：只拦截不执行
}

// ExecutePending 在 send_group_msg 的原时序点执行动作码（时序零变化）。
func ExecutePending(pendings []PendingAction, d *Deps, eventID *string) []ExecOutcome {
	if len(pendings) == 0 {
		return nil
	}
	outcomes := make([]ExecOutcome, 0, len(pendings))
	for _, p := range pendings {
		if d == nil || d.RunAction == nil {
			mylog.Printf("[cqparse] RunAction 未注入,动作未执行: %s", p.Action)
			outcomes = append(outcomes, ExecOutcome{})
			continue
		}
		outcomes = append(outcomes, d.RunAction(p, eventID))
	}
	return outcomes
}

// errDepsNotInjected 统一的依赖缺失错误。
var errDepsNotInjected = errors.New("cqparse: 依赖未注入")

// errUnsafeLocalPath 本地路径包含非法字符（..）。
var errUnsafeLocalPath = errors.New("cqparse: 路径包含非法字符: ..")
