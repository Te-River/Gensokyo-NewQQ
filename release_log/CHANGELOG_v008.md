# Changelog — Release008

> 自 Release007 (`a44882c`) 以来的所有变更。

---

## 🚀 新增功能

### CQ:file 文件上传支持

QQ 机器人 API v2 现已开放 `file_type=4`（文件）的富媒体上传，可通过 `POST /v2/users/{openid}/files` 和 `POST /v2/groups/{group_openid}/files` 上传任意文件。

Gensokyo 新增 `[CQ:file]` CQ 码的完整支持：

- `[CQ:file,file=file:///path/to/file]` — 本地文件路径（自动 base64 编码后走 CDN 上传）
- `[CQ:file,file=http://example.com/file.zip]` — HTTP 远程文件链接
- `[CQ:file,file=https://example.com/file.zip]` — HTTPS 远程文件链接
- `[CQ:file,file=base64://<data>]` — base64 编码数据（走 CDN 上传）

支持可选参数 `file_name` 指定文件名，预留给未来 API 开放使用：
- `[CQ:file,file=file:///path/to/file,file_name=myfile.txt]`
- 数组段格式：`{"type":"file","data":{"file":"file:///path","file_name":"myfile.txt"}}`
- 不填时自动从路径/URL 末尾提取（`filepath.Base()`）

支持场景：
- 群聊发送文件（`send_group_msg`）
- 私聊发送文件（`send_private_msg`）
- 私聊互动召回消息（`send_private_msg_wakeup`）

> 💡 **文件名：** 使用 `file_data`（base64）上传时，`file_name` 参数会直接写入 QQ Bot API 的 `file_name` 字段使文件正确命名。URL 方式发送时 QQ 默认从 URL 末尾提取文件名，也可用 `file_name` 覆盖。

### send_private_msg_wakeup API

新增 `send_private_msg_wakeup` API，用于向 QQ 用户发送 C2C 互动召回（唤醒）消息。OneBot 应用端可通过此接口主动唤醒用户会话，不受被动回复上下文限制。

### 统一图床包 `imagehosting`（oss_type 4~10）

新增 `imagehosting/` 统一图床包，提供 7 种后端：

| 后端 | oss_type | 说明 |
|------|----------|------|
| COS 自签 | 4 | 腾讯云 COS（HMAC-SHA1 自签，无需 SDK） |
| Bilibili | 5 | B站开放平台图片上传（需 Cookie） |
| QQ频道 | 6 | 通过发消息获取 qpic.cn 链接 |
| ChatGLM | 7 | 智谱免费图床，开箱即用 |
| Ukaka | 8 | 免费图床，开箱即用 |
| 星野 | 9 | 免费图床，开箱即用 |
| Nature | 10 | 腾讯 COS 直传（密钥内置），开箱即用 |

- `config/config.go` 新增 `OssTypeCOS` ~ `OssTypeNature` 常量（4~10）
- `imagehosting/hosting.go` 提供 `UploadProvider(name, data, filename)` 统一入口
- `images/upload_api.go` 中 `UploadBase64ImageToServer` 按 `oss_type` 分发到对应后端
- `template/config_template.go` 新增 `image_hosting` 示例段

### 配置模板 `image_hosting` 段

`config_template.go` 新增完整的 `image_hosting` 配置段，涵盖 7 种后端的示例字段。

### 群消息事件新增 `is_private` 字段

`Processor/ProcessC2CMessage.go` 中群消息事件上报新增 `is_private` 布尔字段，标识该消息是否来自群内私聊（`group_private`）场景，方便应用端区分。

---

## 🔧 改进

### send_private_msg_wakeup 处理优化

- 添加 `active` / `active_type` / `active_sub_type` key 遍历跳过逻辑，避免 `[CQ:active]` 内容被错误地作为媒体消息发送
- 纯 `[CQ:active]` 无实际内容时发送空白唤醒请求，确保用户收到互动通知
- 调用方获得真实的成功/失败返回（同步模式），不再因异步处理导致 WebSocket 超时

### 配置自动补全增强

- **YAML 完整块提取** — `extractMissingConfigLines` 现提取完整的 YAML 块（含注释和子字段），而非单行，避免补全后 yaml 结构错误
- **跳过已存在父块的子 key** — `extractMissingConfigLines` 检测到父块已在配置中时，跳过其子 key 的补全，防止重复插入
- **祖先追溯逻辑** — `buildParentKeyMap` 支持多层祖先追溯，确保嵌套配置补全位置正确
- **bool/int 类型支持** — `appendToConfigFile` 补充对 `bool` 和 `int` 类型配置项的补全支持
- **parent=settings 边界修复** — 修复 `parent=settings` 边界情况的插入位置判断

### C2C 消息改为标准 OneBot V11 私聊格式上报

`Processor/ProcessC2CMessage.go` 重构，C2C 消息（私聊）现在以 OneBot V11 标准的 `private_message` 事件格式上报，而非 `message` 格式。`self_id` 字段使用 `int64` 类型。

### 文档完善

- `readme.md` 功能列表/CQ 码/API/鸣谢按最新状态更新，移除 "todo,正在施工..."
- `template/config_template.go` 中 `text_intent` 按模板顺序排列，补全所有 intents 列表
- `image_hosting` 注释完善，标注 oss_type 对应关系
- 精简 readme 鸣谢列表，删除无用鸣谢
- 替换配置示例为实际可用示例

---

## 🐛 Bug 修复

### 语音 URL 未正确重命名引起的潜在 panic

**文件：** `handlers/send_group_msg.go`

`url_record` 分支中 `generateGroupMessage` 使用了外层作用域的 `imageURLs` 变量而非 `recordURLs`。编译通过但在实际语音发送路径下会因索引越界 panic。已修正为 `recordURLs[0]`。

### send_private_msg_wakeup 遍历 active key 时误发媒体

`foundItems` 遍历时未跳过 `"active"` key，导致 `[CQ:active]` 标记被当作媒体类型发送，产生 `"Expected RichMediaMessage type for key active"` 错误。已添加 key 过滤。

### NoneBot 插件 msg_text 截断

**文件：** `active_msg/__init__.py`

`handle_wakeup` 中 `on_command("唤醒")` 已将命令前缀剥离，但代码中仍使用 `text[len("唤醒"):].strip()` 再次裁剪，导致 `"唤醒 @target 123"` 的实际消息变为 `"3"`。已修复为直接使用 `text`。

### 富媒体消息 FileType 注释更新

`botgo/dto/message_create.go` 中 `RichMediaMessage.FileType` 注释新增 `4 文件` 类型。

### 文件消息段未处理导致静默丢弃

**文件：** `handlers/message_parser.go`

NoneBot 以 koishi 数组段格式 `{"type":"file","data":{"file":"file:///..."}}` 发送文件消息时，`parseMessageContent` 的 `switch segmentType` 中没有 `case "file":`，日志打印 `Unhandled segment type: file`，文件被静默丢弃。已在 koishi 和 TRSS 两种消息格式中均添加 `case "file":` 处理。

### 本地文件路径 URL 编码未解码

**文件：** `handlers/message_parser.go`

`file:///` 路径中的中文等字符经 URL 编码（如 `%E7%A5%9E` → `神`），去掉 `file:///` 前缀后路径仍为编码状态，`os.ReadFile` 找不到文件。已在两处 `case "file":` 中添加 `neturl.PathUnescape()` 解码。

### foundItems 遍历缺少文件类型 key

**文件：** `handlers/send_group_msg.go`、`handlers/send_private_msg.go`

`local_file` / `base64_file` 经 `generateGroupMessage` 上传 CDN 后返回 `MessageToCreate`，但遍历 `foundItems` 时 `keyMap` 中没有这些 key，导致上传成功的文件不会被发送。已添加 `local_file`、`url_file`、`url_files`、`base64_file` 到 `keyMap`。

### Markdown 消息中文本段的 [CQ:at] 未合并到 Markdown 内容

**文件：** `handlers/send_group_msg.go`

当后端以数组段格式发送 `[CQ:at]`（如 `{"type":"at","data":{"qq":"121777621"}}`）时，`[CQ:at,qq=数字]` 出现在 `messageText` 中而非 Markdown JSON 内部。但代码只从 `messageText` 中提取已转换后的 `<qqbot-at-user>` 标签，此时 `messageText` 中的 `[CQ:at]` 尚未经 `ResolveMarkdownAtMentions` 转换，`atTag` 始终为空，@ 标签丢失。已修复：在提取标签前对 `messageText` 也调用 `ResolveMarkdownAtMentions`，将 `[CQ:at]` 转换后再合并到 Markdown 内容头部。

### Markdown 图片本地文件路径重写

**文件：** `handlers/message_parser.go`

Markdown 内容中的 `![](本地路径)` 图片在自动上传到 QQ CDN 后，仅替换了 `http://` 和 `https://` 的 URL 模式，未正确处理 `file:///` 协议和纯本地路径。修复后三种路径均能正确识别并替换为 CDN 直链。

### 文件 URL 未正确替换

**文件：** `handlers/message_parser.go`

部分语法中的文件 URL（如 `file:///C:/path/image.png`）在重写 Markdown 图片时未被正确识别和替换，导致图片显示为空白。已修复正则匹配逻辑。

### 配置自动补全：提取完整 YAML 块

`extractMissingConfigLines` 逐行提取时遇到多行值（如 `post_*` 数组）只取首行，导致 YAML 结构损坏。已改为提取从 key 行到下一同缩进 key 行之间的完整块。

### 配置自动补全：跳过已存在父块的子 key

`appendToConfigFile` 在祖先块（如 `settings:`）已存在时仍插入其子 key，导致重复。已添加 `keyExistsInConfig` 检查，跳过已存在的父块。

### 配置自动补全：祖先追溯逻辑

`appendToConfigFile` 使用 `lastIndex` 定位插入点，在配置项无显式父块缩进时错误插入到文件末尾。已改为从目标 key 行向上追溯找到父块进行插入。

### Markdown 图片重写：本地文件路径支持

`ResolveMarkdownImages` 中正则 `!\[.*?\]\((.*?)\)` 未匹配本地路径（无协议头），导致本地 Markdown 图片无法上传。已修复正则并添加 `file://` 协议头补充逻辑。

### 部分语法中的文件 URL 无法被正确替换

修复 Markdown 图片重写中的路径解析问题，确保 `file:///` 和纯本地路径均能被正确上传并替换为 CDN 直链。

### markdown 消息中 [CQ:at] 未转为 QQ @ 标签

**文件：** `handlers/send_group_msg.go`

当 markdown 消息中同时包含 `[CQ:at]` 时，`[CQ:at]` 未被转换为 `<qqbot-at-user>` 标签，导致 `@` 在 markdown 渲染中丢失。已添加 `ResolveMarkdownAtMentions` 调用。

### messageText 中的 [CQ:at] 未合并到 markdown 内容

**文件：** `handlers/send_group_msg.go`

数组段格式发送的 `[CQ:at]` 出现在 `messageText` 中，但合并到 markdown 内容前未先经 `ResolveMarkdownAtMentions` 转换，导致 `atTag` 始终为空。已修复转换顺序。

### 回复消息未设置 msg_id 导致 v2 API 识别失败

**文件：** `handlers/send_group_msg.go`、`handlers/send_private_msg.go`、`handlers/send_guild_channel_msg.go`

`[CQ:reply]` 处理时仅设置了 `MessageReference` 未同时设置 `MsgID`，导致 QQ Bot v2 API 在某些场景下无法正确识别为回复消息。已为三个 Handler 的回复处理均补充 `MsgID` 设置。

### 消息段格式（TRSS）缺少 reply 和 avatar 字段处理

**文件：** `handlers/message_parser.go`

TRSS 格式 `map[string]interface{}` 分支中缺少 `case "reply"` 和 `case "avatar"`，导致该格式下的回复引用和头像转图片功能不可用。已补充。

### [CQ:video] 缺少 base64 和本地文件正则

**文件：** `handlers/message_parser.go`

string 格式（复古 CQ 码）中 `[CQ:video,file=base64://...]` 和 `[CQ:video,file=file://...]` 无对应正则匹配，视频被留在 messageText 中作为普通文本发送。已新增 `base64VideoPattern` 和 `localVideoPattern`。

### foundItems 中 "embed" 名存实亡

**文件：** `handlers/send_group_msg.go`、`handlers/send_private_msg.go`、`handlers/send_private_msg_wakeup.go`

`keyMap` 中包含 `"embed"` key，但没有任何代码写入 `foundItems["embed"]`，且 generate 函数中无 embed 处理分支。若未来误写入会导致空消息发送。已从三个 keyMap 中移除。

### unknown_image/unknown_record/unknown_file 静默丢弃

**文件：** `handlers/message_parser.go`、`handlers/send_group_msg.go`

无前缀的图片/语音/文件 CQ 码（如 `[CQ:image,file=filename.png]`）被收集到 `unknown_*` 后没有任何消费逻辑，静默丢弃。已在 `generateGroupMessage` 和 `generatePrivateMessage` 中添加 fallback 处理，作为 URL 媒体尝试发送。

---

## 🔧 配置变更

- `image_hosting` 配置段展平，所有后端凭证统一放置在 `image_hosting` 下
- `text_intent` 按模板顺序排列，补全完整的 11 个 intent

---

## 📝 文档

- `readme.md` — 功能列表新增 `[CQ:file]`、`send_private_msg_wakeup`、imagehosting 等；CQ 码/API/Event 表格同步最新实现；配置示例替换为完整可用版本；鸣谢更新
- `docs/cq码/标准CQ码/标准cq码-cq-file.md` — 新增 CQ:file 使用文档
- `docs/cq码/扩展CQ码/扩展cq码-cq-at.md` — 更新 markdown 中 @ 标签说明
- `docs/文档-新增功能.md` — 更新 C2C 私聊格式说明
- `imagehosting/README.md` — 图床文档，配置块上移到对应云厂商配置旁
- `release_log/CHANGELOG_v007.md` — 补充 `[CQ:active]` 条目
- `release_log/CHANGELOG_v008.md` — 本文档

---

## 📦 文件变更清单

| 文件 | 变更 |
|------|------|
| `botgo/dto/message_create.go` | FileType 注释新增 `4 文件` 类型；新增 `file_name` 字段 |
| `config/config.go` | 新增 OssTypeCOS ~ OssTypeNature 常量（4~10）；配置自动补全重构；展平 image_hosting 配置 |
| `structs/structs.go` | 新增 ImageHostingConfig 及子结构体 |
| `template/config_template.go` | 新增 `image_hosting` 配置段；text_intent 按序排列 |
| `imagehosting/hosting.go` | 新增统一图床调度器 + 辅助函数 |
| `imagehosting/cos.go` | 新增，腾讯云 COS 自签上传 |
| `imagehosting/bilibili.go` | 新增，B站图床上传 |
| `imagehosting/qq_channel.go` | 新增，QQ频道图床上传 |
| `imagehosting/chatglm.go` | 新增，智谱免费图床 |
| `imagehosting/signed.go` | 新增，Ukaka + 星野签名上传 |
| `imagehosting/nature.go` | 新增，Nature 内置密钥 COS 直传 |
| `imagehosting/utils.go` | 新增，辅助函数 |
| `imagehosting/README.md` | 图床文档，配置块上移 |
| `images/upload_api.go` | 按 oss_type 分发到 imagehosting 后端；增加 file_name 透传 |
| `handlers/message_parser.go` | 新增 CQ:file 正则解析 + foundItems key 映射 + 数组段 `case "file"` + URL 解码；Markdown 图片重写支持本地路径；新增 `base64VideoPattern`/`localVideoPattern` 正则；TRSS 格式补充 reply/avatar 分支 |
| `handlers/send_group_msg.go` | `generateGroupMessage` 文件处理分支 + keyMap 补充 + 文件名传递；回复消息补充 msg_id；keyMap 移除 embed；新增 unknown 类型 fallback |
| `handlers/send_private_msg.go` | keyMap 补充文件类型 + RichMediaMessage 上传后文件名透传；回复消息补充 msg_id；keyMap 移除 embed；新增 unknown 类型 fallback |
| `handlers/send_private_msg_wakeup.go` | 同步模式改造 + active key 跳过 + 空内容兜底；keyMap 移除 embed |
| `handlers/send_guild_channel_msg.go` | 回复消息补充 msg_id |
| `Processor/ProcessC2CMessage.go` | C2C 消息改为标准私聊格式上报；新增 `is_private` 字段 |
| `Processor/Processor.go` | 新增 `is_private` 字段传递 |
| `url/shorturl.go` | 展平 image_hosting 配置适配 |
| `server/getIDHandler.go` | 新增 |
| `main.go` | 展平 image_hosting 配置适配 |
| `AGENTS.md` | 更新 |
| `readme.md` | 功能列表/CQ码/API/Event/鸣谢全面更新 |
| `docs/文档-新增功能.md` | 更新 C2C 私聊格式说明 |
| `docs/cq码/扩展CQ码/扩展cq码-cq-at.md` | 更新 markdown 中 @ 标签说明 |
| `docs/cq码/标准CQ码/标准cq码-cq-file.md` | 新增 CQ:file 使用文档 |
| `release_log/CHANGELOG_v008.md` | 本文档 |

---

## ✅ 提交记录

```
a44882c feat: [CQ:active] 实现主动消息识别
0a826ce docs: CHANGELOG_v007 新增 [CQ:active]
e7e14f4 fix: send_private_msg_wakeup异步处理避免超时
0cc732f 修复文档
664d0ca fix: vet unreachable code + url_record use-after-rename bug
45a36e8 fix: send_private_msg_wakeup遍历时跳过active key
2061a3d fix: send_private_msg_wakeup纯[CQ:active]时发送空唤醒请求
65f6bdd fix: send_private_msg_wakeup立即回送echo避免超时
7c1bf5b fix: 恢复send_private_msg_wakeup同步模式+修复plugin msg_text截断bug
51b00ca feat: CQ:file 文件上传支持+多个bug修复
f63b86c feat: CQ:file 支持可选 file_name 参数
e5e2d41 docs: 新增 CQ:file 标准CQ码文档
57e1499 fix: RichMediaMessage 增加 file_name 字段，上传时传递文件名
3dd7998 docs: file_name 已实际生效，更新文档和changelog
e647af9 feat: 新增 imagehosting 统一图床包 + 删除无用鸣谢
b2cf05b 更新readme
35138d5 docs: 更新 readme 功能列表/CQ码/API/鸣谢
a56a09e docs: 替换配置示例为实际可用示例
80f0348 docs: 替换为完整配置示例
4b203cb feat: 配置模板添加 image_hosting 段
bee6778 docs: 补充完整 text_intent 列表
9d89e79 docs: 按模板顺序排列 text_intent
5ba527a docs: 完善 image_hosting 注释
129d659 fix: 配置自动补全提取完整YAML块而非单行
a549670 fix: 配置自动补全跳过已存在父块的子key
3567594 fix: 配置自动补全祖先追溯逻辑
0d130d8 修复部分语法中的文件url无法被正确替换的错误
37b2180 Fix markdown image rewrite for local file paths
b4e3dce feat: 展平 image_hosting 配置 + 安全修复 P0-P3 + AGENTS.md
707464d 更新agents.md
a061645 docs: 将图床配置块上移到对应云厂商配置旁
aa370b5 feat: 群消息事件新增 is_private 字段标识私聊来源
6f47fbd fix: C2C 消息改为标准 OneBot V11 私聊格式上报
ceb2383 fix: 配置自动补全遗漏 bool/int 类型和 parent=settings 边界情况
8fd955c fix: markdown 消息中 [CQ:at] 未转为 QQ @ 标签
f0f1e35 fix: messageText 中的 [CQ:at] 未合并到 markdown 内容
9640a39 docs: 更新 [CQ:at] 文档及变更日志
c020df6 fix: 修复 CQ 码处理的多项安全缺陷
dae26cc fix: 修复 CQ 码处理的多项需谨慎问题
54ba0ff fix: 修复 CQ 码处理的高风险问题
c7c72b8 fix: 补齐 CQ 码处理的多项边缘问题
215f6bc docs: 更新本版新增功能文档与 Markdown 消息文档
```