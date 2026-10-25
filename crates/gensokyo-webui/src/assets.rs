//! 静态资源：编译期把仓库根 `webui/dist`（前端 `npm run build` 产物）嵌进二进制。
//!
//! 路由规则：
//!
//! - 命中嵌入文件 → 按扩展名给 `Content-Type`；
//! - `/` 与非 API 的 SPA 路径（末段不含 `.`）→ `index.html`；
//! - 形如文件的未命中路径（末段含 `.`）→ 404；
//! - 未注册的 `/api`、`/api/*` → 404，**不回退 `index.html`**（否则前端会把 404 当正常页面）。
//!
//! `webui/dist` 只有 `build.ps1` 的占位文件（还没跑前端构建）时，首页由 [`FALLBACK_INDEX`]
//! 兜底，避免 `/` 直接 404——Go 版同样把整块 `webui/dist` 交给静态服务。

use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// 前端构建产物。目录必须存在（`build.ps1` 的 `Ensure-WebUIDist` / `go:embed` 的同一前提）。
#[derive(RustEmbed)]
#[folder = "../../webui/dist"]
struct Dist;

/// 前端未构建时的兜底首页；真实 `index.html` 存在时永远优先它。
const FALLBACK_INDEX: &str = r#"<!doctype html>
<html lang="zh-CN">
<head><meta charset="utf-8"><title>Gensokyo WebUI</title></head>
<body>
<h1>Gensokyo WebUI</h1>
<p>前端资源尚未构建：请先执行 <code>cd frontend &amp;&amp; npm run build</code>，
产物输出到 <code>webui/dist</code> 后重新编译。</p>
</body>
</html>
"#;

/// 静态资源回退入口（`Router::fallback`）。
pub(crate) async fn serve(method: Method, uri: Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }

    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        // 未注册的 API 路径不能落到 index.html。
        return not_found();
    }
    if let Some(response) = embedded(path) {
        return response;
    }
    if path.is_empty() || !looks_like_file(path) {
        return index();
    }
    not_found()
}

fn embedded(path: &str) -> Option<Response> {
    let asset = Dist::get(path)?;
    Some(file_response(path, asset.data.into_owned()))
}

fn index() -> Response {
    match Dist::get("index.html") {
        Some(asset) => file_response("index.html", asset.data.into_owned()),
        None => file_response("index.html", FALLBACK_INDEX.as_bytes().to_vec()),
    }
}

fn file_response(path: &str, data: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(data));
    let content_type = HeaderValue::from_str(&content_type(path))
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    // index.html 不带内容指纹，禁用缓存：升级二进制后前端外壳立即生效。
    if path == "index.html" {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    response
}

fn content_type(path: &str) -> String {
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    if mime.starts_with("text/") || mime == "application/javascript" || mime == "application/json" {
        format!("{mime}; charset=utf-8")
    } else {
        mime
    }
}

/// 末段带 `.` 视为文件请求（`/app.js`、`/icons/logo.png`）；否则是 SPA 路由。
fn looks_like_file(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "404 Not Found").into_response()
}

#[cfg(test)]
mod tests {
    use super::{content_type, looks_like_file};

    #[test]
    fn content_type_is_derived_from_extension() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("css/app.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("js/app.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("icons/logo.png"), "image/png");
        assert_eq!(content_type("blob.unknownext"), "application/octet-stream");
    }

    #[test]
    fn file_paths_are_told_apart_from_spa_routes() {
        assert!(looks_like_file("js/app.js"));
        assert!(looks_like_file("missing.css"));
        // 只看末段：目录名带点不算文件请求
        assert!(!looks_like_file("a.b/c"));
        assert!(!looks_like_file(""));
        assert!(!looks_like_file("dashboard"));
        assert!(!looks_like_file("dashboard/"));
        assert!(!looks_like_file("/"));
    }
}
