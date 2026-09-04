package dto

// Panel 指令面板 (items≤20, remark≤255 不展示)
type Panel struct {
	Items   []PanelItem `json:"items"`
	Remark  string      `json:"remark,omitempty"`
	Version int64       `json:"version,omitempty"`
}

// PanelItem 面板元素 (name≤14字符, desc≤30字符, type: command|link)
type PanelItem struct {
	Name      string `json:"name"`
	Desc      string `json:"desc,omitempty"`
	Type      string `json:"type"` // command|link
	OnlyAdmin bool   `json:"only_admin,omitempty"`
	Link      string `json:"link,omitempty"` // 仅 type=link
}

// PanelRecord 面板记录
type PanelRecord struct {
	PanelID    string `json:"panel_id"`
	Scope      string `json:"scope"`       // c2c|group|channel|dm
	TargetType string `json:"target_type"` // all|specific
	Panel      *Panel `json:"panel"`
	CreatedAt  string `json:"created_at"` // RFC3339
	UpdatedAt  string `json:"updated_at"` // RFC3339
	Version    int64  `json:"version"`
}

// PanelRecordDetail GET /v2/panels/{panel_id} 响应 = PanelRecord 全字段平铺
// + 关联对象(user_openids/group_openids≤1000, 仅 specific 返回)
type PanelRecordDetail struct {
	PanelID      string   `json:"panel_id"`
	Scope        string   `json:"scope"`
	TargetType   string   `json:"target_type"`
	Panel        *Panel   `json:"panel"`
	CreatedAt    string   `json:"created_at"`
	UpdatedAt    string   `json:"updated_at"`
	Version      int64    `json:"version"`
	UserOpenIDs  []string `json:"user_openids,omitempty"`
	GroupOpenIDs []string `json:"group_openids,omitempty"`
}

// PanelList GET /v2/panels 响应 (next_cursor 空串=末页)
type PanelList struct {
	Records    []PanelRecord `json:"records"`
	NextCursor string        `json:"next_cursor"`
	IsEnd      bool          `json:"is_end"`
}

// CreatePanelRequest POST /v2/panels 请求体
type CreatePanelRequest struct {
	Scope        string   `json:"scope"`                   // 必填
	TargetType   string   `json:"target_type,omitempty"`   // all|specific(仅 c2c/group 支持 specific)
	UserOpenIDs  []string `json:"user_openids,omitempty"`  // ≤20, 仅 c2c specific
	GroupOpenIDs []string `json:"group_openids,omitempty"` // ≤20, 仅 group specific
	Panel        *Panel   `json:"panel"`                   // 必填
}

// CreatePanelResponse POST /v2/panels 响应
type CreatePanelResponse struct {
	PanelID string `json:"panel_id"`
}

// UpdatePanelRequest PUT /v2/panels/{panel_id} 请求体 {panel:Panel 覆盖元素与备注,不影响关联对象}
type UpdatePanelRequest struct {
	Panel *Panel `json:"panel"`
}

// PanelVersionResponse PUT /v2/panels/{panel_id} 响应
type PanelVersionResponse struct {
	Version int64 `json:"version"`
}

// PanelTargetRequest PUT /v2/panels/{panel_id}/target 请求体 (op: add|del, 各≤20, 响应无)
type PanelTargetRequest struct {
	Op           string   `json:"op"`
	UserOpenIDs  []string `json:"user_openids,omitempty"`
	GroupOpenIDs []string `json:"group_openids,omitempty"`
}
