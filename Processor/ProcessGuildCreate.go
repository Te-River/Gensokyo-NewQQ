// 处理收到的信息事件
package Processor

import (
	"encoding/json"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
)

// GuildCreateNotice 频道创建（机器人加入频道）的 OneBot notice 结构
type GuildCreateNotice struct {
	PostType     string `json:"post_type"`
	NoticeType   string `json:"notice_type"`
	SubType      string `json:"sub_type,omitempty"`
	GroupID      int64  `json:"group_id"`
	Time         int64  `json:"time"`
	SelfID       int64  `json:"self_id"`
	RealGroupID  string `json:"real_group_id,omitempty"` // 频道真实 ID
	GuildName    string `json:"guild_name,omitempty"`    // 频道名称
	OperatorID   int64  `json:"operator_id,omitempty"`   // 操作人虚拟 ID
	MemberCount  int    `json:"member_count,omitempty"`  // 频道成员数
	MaxMembers   int64  `json:"max_members,omitempty"`   // 频道成员上限
	Description  string `json:"description,omitempty"`   // 频道简介
}

// ProcessGuildCreate 处理频道创建事件（机器人被加入到某个频道时触发，GUILD_CREATE）
func (p *Processors) ProcessGuildCreate(data *dto.WSGuildData) error {
	if data == nil {
		mylog.Printf("ProcessGuildCreate: 数据为空")
		return nil
	}

	selfID := int64(config.GetAppID())

	// 将 guild_id 转为虚拟 group_id
	groupID, err := idmap.StoreIDv2(data.ID)
	if err != nil {
		mylog.Printf("ProcessGuildCreate: guild_id 转换失败: %v", err)
		return nil
	}

	// 操作用户（将机器人拉入频道的用户）
	var operatorID int64
	if data.OpUserID != "" {
		operatorID, err = idmap.StoreIDv2(data.OpUserID)
		if err != nil {
			mylog.Printf("ProcessGuildCreate: op_user_id 转换失败: %v", err)
		}
	}

	notice := GuildCreateNotice{
		PostType:    "notice",
		NoticeType:  "guild_create",
		GroupID:     groupID,
		Time:        time.Now().Unix(),
		SelfID:      selfID,
		RealGroupID: data.ID,
		GuildName:   data.Name,
		OperatorID:  operatorID,
		MemberCount: data.MemberCount,
		MaxMembers:  data.MaxMembers,
		Description: data.Desc,
	}

	mylog.Printf("频道创建: guild=%s, name=%s, members=%d", data.ID, data.Name, data.MemberCount)

	// 序列化为 map 并广播
	jsonData, _ := json.Marshal(notice)
	var outputMap map[string]interface{}
	json.Unmarshal(jsonData, &outputMap)

	go p.BroadcastMessageToAll(outputMap, p.Apiv2, data)
	return nil
}
