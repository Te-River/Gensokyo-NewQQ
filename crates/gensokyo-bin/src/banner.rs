//! 启动横幅：对齐 Go `main.go:663-667` 的欢迎语与 `template/config_template.go:285` 的 `Logo`。
//!
//! # 与 Go 的差异
//! - Go 用 `color.New(color.FgCyan)` 无条件染色；这里只在 stdout 是终端时加 ANSI 青色
//!   （[`std::io::IsTerminal`]），重定向到日志文件时不混入转义序列。
//! - Go 的窗口标题走 `settings.title` + `SetConsoleTitle`；Rust 没有对应 API，[`TITLE`]
//!   作为横幅首行打印，值本身保持可见。
//! - Go `ComposeWebUIURLv2` 联网查公网 IP；Rust 启动期不联网，公网地址只按配置
//!   `server_dir:port` 拼，未配置时该行给出占位说明而不是编造地址。
//! - Go 的地址带 `/webui` 前缀；Rust 的 WebUI 由 `gensokyo_webui::WebUi::router` 的
//!   `fallback(assets::serve)` 挂在 `/`，所以这里给根路径。

use std::io::IsTerminal;
use std::net::SocketAddr;

use gensokyo_config::Settings;

/// 程序标题（Go `settings.title` 的等价物；Rust 无控制台标题 API，打印成横幅首行）。
pub const TITLE: &str = "Gensokyo-NewQQ - Te-River";

/// 东方群像字符画，逐字节取自 Go `template/config_template.go` 的 `const Logo`：首行空行、
/// 每行 `'` 前缀、行尾填充空格与各行宽度（前十行 108 列、末行 127 列）都原样保留（LF 换行）。
///
/// 扩展名用 `.art` 而非 `.txt`：仓库 `.gitignore:33` 忽略 `*.txt`，改名可以不改忽略规则。
pub const LOGO: &str = include_str!("logo.art");

/// 青色（ANSI SGR 36）。
const CYAN: &str = "\x1b[36m";
/// 复位（ANSI SGR 0）。
const RESET: &str = "\x1b[0m";

/// WebUI 未启用时的控制台地址占位。
const WEBUI_OFF: &str = "（WebUI 未启用，disable_webui=true）";
/// `server_dir` 未配置（或仍是模板占位符）时的公网地址占位。
const NO_SERVER_DIR: &str = "（未配置 server_dir，仅本机可访问）";

/// 打印横幅：标题 → 本地控制台地址 → Logo → 公网控制台地址。
///
/// 两个地址为 `None` 时该行打印占位说明：启动期不猜测地址，避免把打不开的 URL 摆给用户。
pub fn welcome(console_url: Option<&str>, public_url: Option<&str>) {
    let cyan = std::io::stdout().is_terminal();
    emit(TITLE, cyan);
    emit(
        &format!(
            "欢迎来到Gensokyo-NewQQ, 控制台地址: {}",
            console_url.unwrap_or(WEBUI_OFF)
        ),
        cyan,
    );
    // `LOGO` 自带首尾换行：`println!` 再多一个换行，与 Go `Printf("%s\n", Logo)` 一致。
    emit(LOGO, cyan);
    emit(
        &format!(
            "欢迎来到Gensokyo-NewQQ, 公网控制台地址(需开放端口): {}",
            public_url.unwrap_or(NO_SERVER_DIR)
        ),
        cyan,
    );
}

/// 输出一段横幅文本：终端上整段青色，管道/文件里纯文本。
fn emit(text: &str, cyan: bool) {
    if cyan {
        println!("{CYAN}{text}{RESET}");
    } else {
        println!("{text}");
    }
}

/// 控制台地址：`webui` 是实际承载 WebUI 的 listener（WebUI 未启用或没起 listener 时为 `None`）。
///
/// 本地地址以**真实绑定**端口为准——Go `ComposeWebUIURL` 按配置 `server_dir:port` 拼，
/// 端口被 `RuntimeOptions` 覆盖时 Go 打印的是旧端口；这里用 `SocketAddr` 不会过期。
/// 公网地址按配置 `server_dir:port` 拼（Go `ComposeWebUIURLv2` 那一步需要联网）。
///
/// Go 在端口为 443 时把协议换成 https；Rust 启动链还没有 TLS listener，这里固定 http。
#[must_use]
pub fn console_urls(
    webui: Option<SocketAddr>,
    settings: &Settings,
) -> (Option<String>, Option<String>) {
    let console = webui.map(|addr| {
        let host = if addr.ip().is_unspecified() {
            // 绑 0.0.0.0/:: 时浏览器打不开，落到回环地址，与 Go 默认的 server_dir 一致。
            "127.0.0.1".to_owned()
        } else {
            addr.ip().to_string()
        };
        format!("http://{host}:{}/", addr.port())
    });
    let server_dir = settings.get_server_dir().trim();
    let port = settings.get_port_value().trim();
    let public = (!server_dir.is_empty() && !server_dir.starts_with('<') && !port.is_empty())
        .then(|| format!("http://{server_dir}:{port}/"));
    (console, public)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use gensokyo_config::Settings;

    use super::{LOGO, TITLE, console_urls};

    /// Logo 与 Go 同形：11 行字符画，每行 `'` 开头，前后各一个换行，宽度逐行对齐。
    #[test]
    fn logo_matches_go_shape() {
        assert!(LOGO.starts_with('\n'), "Go `const Logo` 以换行开头");
        assert!(LOGO.ends_with('\n'), "Go `const Logo` 以换行结尾");
        let art: Vec<&str> = LOGO
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        assert_eq!(art.len(), 11, "Go Logo 是 11 行字符画");
        for line in &art {
            // 首行是纯填充（`'` + 107 个空格），所以只校验前缀，不要求有可见字符。
            assert!(line.starts_with('\''), "每行以 ' 开头: {line:?}");
        }
        assert!(art[..10].iter().all(|line| line.chars().count() == 108));
        assert_eq!(art[10].chars().count(), 127);
    }

    #[test]
    fn title_names_the_project() {
        assert!(TITLE.contains("Gensokyo-NewQQ"));
    }

    /// 本地地址取真实 listener：未指定地址落到回环，WebUI 挂在 `/`。
    #[test]
    fn console_url_uses_live_listener() {
        let (console, public) =
            console_urls(Some("0.0.0.0:5700".parse().unwrap()), &Settings::default());
        assert_eq!(console.as_deref(), Some("http://127.0.0.1:5700/"));
        assert_eq!(public, None, "server_dir 未配置时不给公网地址");
    }

    #[test]
    fn console_urls_read_server_dir() {
        let settings = Settings {
            server_dir: "example.com".to_owned(),
            port: "15630".to_owned(),
            ..Settings::default()
        };
        let addr: SocketAddr = "192.0.2.10:15630".parse().unwrap();
        let (console, public) = console_urls(Some(addr), &settings);
        assert_eq!(console.as_deref(), Some("http://192.0.2.10:15630/"));
        assert_eq!(public.as_deref(), Some("http://example.com:15630/"));
    }

    /// 模板占位符 `<YOUR_SERVER_DIR>` 不能当成公网地址打印。
    #[test]
    fn placeholder_server_dir_is_ignored() {
        let settings = Settings {
            server_dir: "<YOUR_SERVER_DIR>".to_owned(),
            port: "15630".to_owned(),
            ..Settings::default()
        };
        let (console, public) = console_urls(None, &settings);
        assert_eq!(console, None);
        assert_eq!(public, None);
    }
}
