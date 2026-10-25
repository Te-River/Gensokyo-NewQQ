//! 版本串。
//!
//! 与 Go `buildinfo.Version()` 逐字一致：构建期注入 `BuildType-BuildSpec`，未注入时回退
//! `dev-<epoch>.<millis>`（UTC 秒 + 毫秒）。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前二进制的版本串。
#[must_use]
pub fn version_string() -> String {
    match option_env!("GENSOKYO_BUILD_VERSION") {
        Some(build) if !build.is_empty() => build.to_owned(),
        _ => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            format!("dev-{}.{:03}", now.as_secs(), now.subsec_millis())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::version_string;

    #[test]
    fn injected_version_wins_otherwise_dev_prefix() {
        let version = version_string();
        match option_env!("GENSOKYO_BUILD_VERSION") {
            Some(injected) if !injected.is_empty() => assert_eq!(version, injected),
            _ => assert!(version.starts_with("dev-"), "版本串: {version}"),
        }
    }
}
