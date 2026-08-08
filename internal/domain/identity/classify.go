package identity

// 本文件是"长度启发式"的 legacy 收敛点。
//
// 旧代码中散落的 len(id)==32 / len(id)!=32 身份判断一律改为调用本包函数，
// 使身份推断只存在于转换边界，未来由显式解析器替代。
//
// 注意：QQ OpenID 当前为 32 位字符串。这是兼容旧行为的最小收敛，不是永久方案。

// IsOpenID 报告 s 是否为 QQ OpenID（legacy 语义：长度 32）。
func IsOpenID(s string) bool { return len(s) == 32 }

// IsVirtualID 报告 s 是否为虚拟数字 ID（legacy 语义：非空纯数字）。
func IsVirtualID(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}
