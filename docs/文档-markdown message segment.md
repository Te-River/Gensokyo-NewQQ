```markdown
# Gensokyo Markdown Segment

Gensokyo的Markdown Segment是对现有OneBot v11的扩展。

## Markdown卡片（文本形式）

```json
{
    "type": "markdown",
    "data": {
        "data": "文本内容"
    }
}
```

| 参数名   | 收 | 发 | 可能的值 | 说明        |
|----------|----|----|----------|-------------|
| data     | ✓  | ✓  | -        | md文本      |

**文本内容为**：
- [`文档-markdown定义.md`](../docs/文档-markdown定义.md) 中 markdown 的 json 字符串的 base64（以 base64:// 开头，文字处理为 /u 形式的 unicode）或按以下规则处理后的，json 实体化文本。

**转义**：
CQ 码由字符 [ 起始, 以 ] 结束, 并且以 , 分割各个参数。如果你的 CQ 码中, 参数值包括了这些字符, 那么它们应该被使用 HTML 特殊字符的编码方式进行转义。

字符 | 对应实体转义序列
-----|------------------
&    | &amp;
[    | &#91;
]    | &#93;
,    | &#44;

## Markdown卡片（object形式）

```json
{
    "type": "markdown",
    "data": {
        "data": md object
    }
}
```

| 参数名   | 收 | 发 | 可能的值 | 说明        |
|----------|----|----|----------|-------------|
| data     | ✓  | ✓  | -        | md object   |

**结构请参考**：
支持MessageSegment（参考 [`文档-markdown定义.md`](../docs/文档-markdown定义.md)）与文本形式实际包含内容相同，但传参类型不同，不是string，而是你所组合的md卡片object（map）。

data下层应包含data（2层data），data.markdown，data.keyboard。
同时与type同级的data字段是OneBot v11标准固定的，所以json结构会呈现data.data.markdown，data.data.keyboard双层结构。
```

> **📌 相关内容**
> - [Markdown 消息定义](./文档-markdown定义.md) — 自定义/模板 MD 格式
> - [Embed 消息定义](./文档-embed消息定义.md) — 结构化卡片消息
> - [文档索引](./更多文档.md)