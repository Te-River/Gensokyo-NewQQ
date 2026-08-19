// Package qq 是 QQ SDK（botgo）与内部 domain 模型的转换边界。
//
// 规则（P12.1/P12.2）：
//   - botgo 只能 import 到本包（以及 main/bootstrap 组装处）。
//   - domain / application 禁止 import botgo。
//   - 入站：botgo DTO → internal domain model（DomainEvent）
//   - 出站：internal outbound model → botgo request
package qq

import (
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/tencent-connect/botgo/dto"
)

// OpenIDFromBotgoUser 从 botgo dto.User 提取 OpenID（转换边界示例）。
// 只有本包（adapter）允许做 botgo → typed identity 的转换。
func OpenIDFromBotgoUser(u *dto.User) identity.OpenID {
	return identity.OpenID(u.ID)
}

// OpenGroupIDFromBotgo 从 botgo 群 ID 字符串构造群 OpenID（转换边界示例）。
// 入站事件中群 OpenID 以 string 出现，这里收敛为 typed 类型。
func OpenGroupIDFromBotgo(raw string) identity.OpenGroupID {
	return identity.OpenGroupID(raw)
}
