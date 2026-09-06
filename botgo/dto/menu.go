package dto

// Menu 自定义菜单 (items≤10, 仅 C2C 场景全局生效)
type Menu struct {
	Items []MenuItem `json:"items"`
}

// MenuItem 菜单项 (name≤10字符,汉字算2)
type MenuItem struct {
	Name         string        `json:"name"`
	Type         string        `json:"type"`                     // switch|send_message|link|menu
	SubMenuItems []SubMenuItem `json:"sub_menu_items,omitempty"` // 仅 type=menu, ≤5, 不支持再嵌套
	SendMessage  string        `json:"send_message,omitempty"`
	Link         string        `json:"link,omitempty"`   // 必须 https://
	Switch       *MenuSwitch   `json:"switch,omitempty"` // "switch" 是 JSON key,Go 字段名避开关键字
}

// MenuSwitch 开关型菜单项
type MenuSwitch struct {
	SwitchID string `json:"switch_id"`
	Default  bool   `json:"default"`
}

// SubMenuItem 子菜单项 (name≤14字符, type 仅 send_message|link)
type SubMenuItem struct {
	Name        string `json:"name"`
	Type        string `json:"type"`
	SendMessage string `json:"send_message,omitempty"`
	Link        string `json:"link,omitempty"`
}

// MenuResponse GET /v2/menu 响应 (未设置过时 Menu 为空)
type MenuResponse struct {
	Version int64 `json:"version"`
	Menu    *Menu `json:"menu"`
}

// PutMenuRequest PUT /v2/menu 请求体 {menu:Menu 覆盖式}
type PutMenuRequest struct {
	Menu *Menu `json:"menu"`
}

// MenuVersionResponse PUT /v2/menu 响应
type MenuVersionResponse struct {
	Version int64 `json:"version"`
}
