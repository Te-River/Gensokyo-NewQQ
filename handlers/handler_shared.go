package handlers

// checkZeroGroupID 检查 GroupID 是否有效（不为 0 或空）
func checkZeroGroupID(id interface{}) bool {
	switch v := id.(type) {
	case int:
		return v != 0
	case int64:
		return v != 0
	case string:
		return v != "0"
	default:
		return true
	}
}

// checkZeroUserID 检查 UserID 是否有效（不为 0 或空）
func checkZeroUserID(id interface{}) bool {
	switch v := id.(type) {
	case int:
		return v != 0
	case int64:
		return v != 0
	case string:
		return v != "0"
	default:
		return true
	}
}