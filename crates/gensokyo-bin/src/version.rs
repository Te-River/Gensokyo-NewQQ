//! 版本串。
//!
//! 与 Go `buildinfo.Version()` 逐字一致：构建期注入 `<BuildType>-<BuildSpec>`，未注入时
//! 回退 `dev-<epoch>.<millis>`（UTC 秒 + 毫秒）。
//!
//! 注入点只有一处：`cargo xtask build`（`build.ps1` 亦转调它）给 cargo 子进程设
//! `GENSOKYO_BUILD_VERSION`——有 git 时 `git-<7 位短 hash>`，否则 `dev-<epoch>.<millis>`，
//! 与 Go `build.ps1` 的 `Get-BuildLdflags` 同形态；已设的同名环境变量优先，发布流水线
//! 可直接覆盖。直接 `cargo build` 不注入，即走下面的运行时回退。

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
