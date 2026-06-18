# Gensokyo 语法参考

Gensokyo 对 OneBot V11 的扩展语法汇总。

## CQ 码（扩展）

| CQ 码 | 格式 | 适用范围 | 说明 |
|-------|------|:--------:|------|
| Markdown | \[CQ:markdown,data=base64]\ | 🌐 全场景 | Markdown 卡片消息 |
| 头像 | \[CQ:avatar,qq=数字]\ | 🌐 全场景 | 在消息中嵌入用户头像图片 |
| QQ 音乐 | \[CQ:music,type=qq,id=数字]\ | 🌐 全场景 | 分享 QQ 音乐（自动转为 Markdown 卡片） |
| 回复 | \[CQ:reply,id=数字]\ | 📡 仅频道 | 引用回复标记 |
| 成员变动 | \[CQ:member,type=add/remove,user_id=数字]\ | 🏷️ 群聊 | 群成员入群/退群 CQ 码 |
| 主动标记 | \[CQ:active]\ | 🌐 全场景 | 标记主动推送模式 |

> 图例: 🌐 全场景 | 🏷️ 群聊 | 📡 频道 | 💬 C2C
