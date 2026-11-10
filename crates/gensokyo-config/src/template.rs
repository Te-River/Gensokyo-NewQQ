//! 首启模板生成。
//!
//! 模板内容与 Go `template/config_template.go` 的 `ConfigTemplate` 原始字符串一致
//! （`templates/config.yml`，UTF-8）。这里只做"文件缺失时写出模板"；Go
//! `ensureConfigComplete` 的补键回写见 [`crate::complete`]。
//!
//! 写盘走 [`crate::loader::write_atomic`]（同目录 `config.yml.<pid>.tmp` → rename）：进程在
//! 写一半时崩溃不会留下半截 `config.yml`，下次启动仍能重新生成。配置写盘全仓只有这一份
//! temp + rename 实现。

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// 与 Go `ConfigTemplate` 一致的配置模板（编译期嵌入）。
pub const CONFIG_TEMPLATE: &str = include_str!("../templates/config.yml");

/// 首启时替换为本机内网 IP 的占位符（Go `config/config.go:858`）。
const SERVER_DIR_PLACEHOLDER: &str = "<YOUR_SERVER_DIR>";

/// 模板生成错误。
#[derive(Debug, Error)]
pub enum TemplateError {
    /// 创建配置所在目录失败。
    #[error("创建配置目录 {} 失败: {source}", path.display())]
    CreateDir {
        /// 目录路径。
        path: PathBuf,
        /// 底层 IO 错误。
        #[source]
        source: std::io::Error,
    },
    /// 写入模板失败。
    #[error("写入配置模板 {} 失败: {source}", path.display())]
    Write {
        /// 配置路径。
        path: PathBuf,
        /// 底层 IO 错误。
        #[source]
        source: std::io::Error,
    },
}

/// 配置不存在时写出模板，返回是否**新建**了文件；已存在则原样保留。
pub fn ensure_config_file(path: impl AsRef<Path>) -> Result<bool, TemplateError> {
    let path = path.as_ref();
    if path.exists() {
        return Ok(false);
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|source| TemplateError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    match crate::loader::write_atomic(path, &render_template()) {
        Ok(()) => Ok(true),
        // 目标在此期间被别的进程创建：按"已存在、未生成"处理（rename 失败原因不重要了）
        Err(_) if path.exists() => Ok(false),
        Err(source) => Err(TemplateError::Write {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// 生成模板内容：`<YOUR_SERVER_DIR>` 换成本机内网 IP（对齐 Go `config.go:858`）。
///
/// 取不到内网 IP 时保留占位符并 warn，让用户手工填写，而不是生成一个空地址。
fn render_template() -> String {
    match gensokyo_core::local_ip() {
        Some(ip) => CONFIG_TEMPLATE.replace(SERVER_DIR_PLACEHOLDER, &ip.to_string()),
        None => {
            crate::warn_early(
                "未能获取本机内网 IP，config.yml 中的 <YOUR_SERVER_DIR> 保留占位符，请手工填写",
            );
            CONFIG_TEMPLATE.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目录里除 `config.yml` 之外的残留文件名（临时文件必须在成功后被清掉）。
    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .expect("读目录失败")
            .map(|entry| {
                entry
                    .expect("目录项")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name != "config.yml")
            .collect()
    }

    #[test]
    fn template_is_embedded_verbatim() {
        assert!(CONFIG_TEMPLATE.starts_with("version: 1\nsettings:\n"));
        assert!(CONFIG_TEMPLATE.contains("ws_address:"));
        assert!(CONFIG_TEMPLATE.contains(SERVER_DIR_PLACEHOLDER));
        // 中文注释未被转义污染
        assert!(CONFIG_TEMPLATE.contains("反向ws设置"));
        // 多行模板没有被压成一行/转义文本，也没有解码失败替换符
        assert!(CONFIG_TEMPLATE.lines().count() > 200);
        assert!(!CONFIG_TEMPLATE.contains('\u{fffd}'));
    }

    #[test]
    fn creates_missing_file_and_keeps_existing() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("nested").join("config.yml");

        assert!(
            ensure_config_file(&path).expect("生成失败"),
            "缺失时必须生成"
        );

        let generated = fs::read_to_string(&path).expect("读回失败");
        match gensokyo_core::local_ip() {
            Some(ip) => assert_eq!(
                generated,
                CONFIG_TEMPLATE.replace(SERVER_DIR_PLACEHOLDER, &ip.to_string())
            ),
            // 没有内网 IP 的机器保留占位符
            None => assert_eq!(generated, CONFIG_TEMPLATE),
        }
        assert!(
            leftovers(path.parent().expect("父目录")).is_empty(),
            "成功后不得残留 tmp"
        );

        fs::write(&path, "version: 2\n").expect("写测试文件失败");
        assert!(
            !ensure_config_file(&path).expect("二次调用失败"),
            "已存在时不生成"
        );
        assert_eq!(fs::read_to_string(&path).expect("读回失败"), "version: 2\n");
    }

    /// 别的进程（别的 pid）留下的临时文件不在本进程的责任范围：不读它、也不删它。
    #[test]
    fn stale_tmp_from_another_pid_does_not_block_generation() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        let stale = dir.path().join("config.yml.1234.tmp");
        fs::write(&stale, "半截").expect("写 tmp 失败");

        assert!(ensure_config_file(&path).expect("生成失败"));
        assert!(
            fs::read_to_string(&path)
                .expect("读回失败")
                .starts_with("version: 1")
        );
        assert_eq!(fs::read_to_string(&stale).expect("读回失败"), "半截");
    }
}
