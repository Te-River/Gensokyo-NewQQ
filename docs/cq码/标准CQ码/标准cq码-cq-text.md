# [CQ:text]

## 用途

`[CQ:text]` 是消息中最基本的文本部分。在 OneBot V11 协议中，任何消息都可以由一个或多个 `[CQ:text]` 段和其他 CQ 码段组合而成。

范围：`全场景`

## 语法

```text
[CQ:text,text=需要发送的文本]
```

## 参数

| 参数名 | 必填 | 说明 |
|--------|------|------|
| `text` | ✅ | 纯文本内容，支持 CQ 码转义：四个特殊字符 `,` `]` `[` `&` 分别写作 `&#44;` `&#93;` `&#91;` `&amp;` |

## 解析行为

### 字符串模式（string_ob11=false）

在字符串上报模式下，`[CQ:text]` 不会显式出现——文本内容直接作为消息字符串的一部分。例如：

```text
你好[CQ:image,file=xxx]世界
```

其中 `你好` 和 `世界` 就是隐式的文本部分。

### 数组模式（array=true）

在数组上报模式下，文本内容被包装为独立的 `text` 段：

```json
[
  {"type": "text", "data": {"text": "你好"}},
  {"type": "image", "data": {"file": "xxx", "url": "..."}}
]
```

## 发送行为

- **出站字符串模式**：`[CQ:text,text=xxx]` 会被解析为纯文本 `xxx`，与其他 CQ 码拼接后发送。
- **出站数组模式**：`{"type": "text", "data": {"text": "xxx"}}` 段中的文本直接拼接到消息正文。
- 文本中的 URL 会根据 `transfer_url` 配置决定是否转换为短链（默认 `true`，改写与删除的完整口径见 [CQ 码汇总](../CQ码汇总.md) 的「URL 改写与二维码」）。
- 文本中的敏感词会根据 `enableChangeWord` 配置进行替换。

## 使用示例

### NoneBot 插件

```python
from nonebot.adapters.onebot.v11 import Message, MessageSegment

await bot.send_group_msg(
    group_id=123456,
    message=Message([
        MessageSegment.text("你好，世界！"),
    ]),
)
```

### CQ 码字符串格式

```text
[CQ:text,text=你好世界]
```

## 注意

- `[CQ:text]` 的 `text` 参数中如需使用特殊字符 `,` `]` `[` `&`，需进行 CQ 码转义（`&#44;` `&#93;` `&#91;` `&amp;`）。
- 空文本段（`text=` 为空）在解析时会被忽略。

### 转义的编解码顺序（顺序敏感）

唯一实现见 `handlers/cqparse/escape.go:7-24`：

- **编码**：先 `&` → `&amp;`，再 `,` → `&#44;`、`]` → `&#93;`、`[` → `&#91;`。
- **解码**：严格按编码的逆序，`&#93;` → `&#44;` → `&#91;` → 最后才是 `&amp;` → `&`。
- 顺序不可交换：编码先把 `&` 转义，字面量 `&#44;` 会变成 `&amp;#44;`，解码前三步不会误伤它，`&amp;` 留到最后才还原为 `&` 本身；若先还原 `&amp;`，`&amp;#44;` 会被二次解码成 `,`，产生数据损坏。
