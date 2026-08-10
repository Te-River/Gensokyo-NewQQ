# [CQ:music]

## 用途

`[CQ:music]` 用于发送音乐分享消息。Gensokyo 当前仅支持 QQ 音乐（`type=qq`），通过 QQ Bot API 的音乐卡片能力发送。

范围：`q群 (Group Chat)` / `C2C (私聊)`

## 语法

```text
[CQ:music,type=qq,id=歌曲ID]
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `type` | ✅ | 音乐平台类型，当前仅支持 `qq`（QQ 音乐） |
| `id` | ✅ | 歌曲 ID（QQ 音乐的数字 ID） |

## 与标准 OneBot V11 的差异

标准 OneBot V11 支持 `qq`、`163`（网易云）、`xm`（虾米）等多种音乐平台。Gensokyo 由于 QQ Bot API 的限制，**仅支持 QQ 音乐**（`type=qq`）。

| 差异点 | 标准 OneBot V11 | Gensokyo 实现 |
|--------|----------------|---------------|
| QQ 音乐 | ✅ | ✅ 支持 |
| 网易云音乐 | ✅ | ❌ 不支持 |
| 虾米音乐 | ✅ | ❌ 不支持 |
| 自定义音乐 | ✅ `type=custom` | ❌ 不支持 |

## 解析行为

### 字符串模式

`[CQ:music,type=qq,id=123456]` 通过正则 `qqMusicPattern` 匹配，提取 `type` 和 `id`，存入 `foundItems["qqmusic"]`。

### 数组模式

```json
{"type": "music", "data": {"type": "qq", "id": "123456"}}
```

同样提取 `type=qq` 和 `id` 进行处理。

## 发送行为

当 `type=qq` 时，Gensokyo 使用 QQ Bot API 的音乐卡片接口发送：

1. 根据歌曲 ID 构建音乐分享参数
2. 通过 QQ Bot API 发送音乐卡片消息
3. 用户在 QQ 客户端中可直接播放

如果 `type` 不是 `qq`，该 CQ 码会被忽略。

## 使用示例

### NoneBot 插件

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.music("qq", "284973"),  # QQ 音乐歌曲 ID
    ]),
)
```

### CQ 码字符串格式

```text
[CQ:music,type=qq,id=284973]
```

### 配合点歌指令

可配合 `music_prefix` 配置实现点歌功能：

```yaml
music_prefix: "点歌"
```

用户输入 `点歌 稻香` 即可触发点歌流程。

## 注意

- 仅支持 QQ 音乐（`type=qq`），其他平台类型的音乐消息会被忽略。
- 歌曲 ID 为 QQ 音乐的数字 ID，可在 QQ 音乐 App 中分享获取。
- 音乐卡片消息需要机器人具有相应的消息发送权限。
