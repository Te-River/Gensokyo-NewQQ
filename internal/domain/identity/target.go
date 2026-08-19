package identity

// TargetKind 发送目标类型。
type TargetKind uint8

const (
	// TargetGroup 群聊。
	TargetGroup TargetKind = iota + 1
	// TargetPrivate 私聊（C2C）。
	TargetPrivate
)

// ResolvedTarget 解析后的发送目标。
type ResolvedTarget struct {
	Kind  TargetKind
	Group *ResolvedGroup
	User  *ResolvedUser
}

// String 描述目标，便于日志。
func (t ResolvedTarget) String() string {
	switch {
	case t.Kind == TargetGroup && t.Group != nil:
		return "group:" + t.Group.VirtualGroupID.String()
	case t.Kind == TargetPrivate && t.User != nil:
		return "private:" + t.User.VirtualUserID.String()
	default:
		return "unknown"
	}
}
