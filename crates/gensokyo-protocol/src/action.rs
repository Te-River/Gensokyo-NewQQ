//! OneBot V11 action 请求 / 响应模型（出站方向：OneBot 客户端 → gensokyo）。

use serde::{Deserialize, Serialize};

/// 请求回传的 `echo`：字符串或整数，原样返回。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Echo {
    /// 整数 echo。
    Number(i64),
    /// 字符串 echo。
    Text(String),
}

/// 一条 action 请求。
///
/// `params` 在 M0 以泛型占位，M1 起由各 action 的类型化参数结构替换：先锁定一个通用
/// JSON 表示会把 M1 的形状固化错。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActionRequest<P> {
    /// action 名称（含向后兼容别名）。
    pub action: String,
    /// action 参数。
    #[serde(default)]
    pub params: Option<P>,
    /// 原样回传的 echo。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<Echo>,
}

/// 响应状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// 成功。
    Ok,
    /// 失败。
    Failed,
    /// 异步（如 `send_forward_msg`）。
    Async,
}

/// 一条 action 响应。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActionResponse<T = ()> {
    /// 状态。
    pub status: Status,
    /// 返回码，0 为成功。
    pub retcode: i32,
    /// 业务数据：恒序列化（None 输出 `null`，对齐 OneBot 响应结构，不省略字段）。
    #[serde(default)]
    pub data: Option<T>,
    /// 错误信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 补充说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wording: Option<String>,
    /// 原样回传的 echo。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<Echo>,
}

impl<T> ActionResponse<T> {
    /// 成功响应。
    #[must_use]
    pub fn ok(data: T) -> Self {
        Self {
            status: Status::Ok,
            retcode: 0,
            data: Some(data),
            message: None,
            wording: None,
            echo: None,
        }
    }

    /// 失败响应。
    #[must_use]
    pub fn failed(retcode: i32, message: impl Into<String>) -> Self {
        Self {
            status: Status::Failed,
            retcode,
            data: None,
            message: Some(message.into()),
            wording: None,
            echo: None,
        }
    }

    /// 挂上待回传的 echo。
    #[must_use]
    pub fn with_echo(mut self, echo: Option<Echo>) -> Self {
        self.echo = echo;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionResponse, Status};

    #[test]
    fn data_is_always_serialized_even_when_none() {
        // 契约：data 字段恒出现，失败响应为 null（对齐 OneBot 客户端按字段读取的习惯）
        let failed: ActionResponse<String> = ActionResponse::failed(-1, "boom");
        let yaml = serde_norway::to_string(&failed).expect("序列化失败");
        assert!(yaml.contains("data: null"), "data 必须保留为 null: {yaml}");
        assert_eq!(failed.status, Status::Failed);

        let ok: ActionResponse<String> = ActionResponse::ok("x".to_owned());
        let yaml = serde_norway::to_string(&ok).expect("序列化失败");
        assert!(yaml.contains("data: x"), "data 必须带值: {yaml}");
        assert_eq!(ok.status, Status::Ok);
    }
}
