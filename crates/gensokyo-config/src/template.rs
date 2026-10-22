//! 首启模板生成。
//!
//! 模板内容与 Go `template/config_template.go` 的 `ConfigTemplate` 原始字符串一致
//! （`templates/config.yml`，UTF-8）。这里只做"文件缺失时写出模板"，**不做** Go
//! `ensureConfigComplete` 的字段合并——那是 M1d 的范围。
//!
//! 写盘走"同目录 `config.yml.tmp` + rename"：进程在写一半时崩溃不会留下半截
//! `config.yml`，下次启动仍能重新生成。

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::warn;

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

    let tmp = tmp_path(path);
    let mut file = create_tmp(&tmp).map_err(|source| TemplateError::Write {
        path: tmp.clone(),
        source,
    })?;

    let content = render_template();
    if let Err(source) = file
        .write_all(content.as_bytes())
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(TemplateError::Write { path: tmp, source });
    }
    drop(file);

    match fs::rename(&tmp, path) {
        Ok(()) => Ok(true),
        Err(source) => {
            let _ = fs::remove_file(&tmp);
            // 目标在此期间被别的进程创建：按"已存在、未生成"处理
            if path.exists() {
                return Ok(false);
            }
            Err(TemplateError::Write {
                path: path.to_path_buf(),
                source,
            })
        }
    }
}

/// 生成模板内容：`<YOUR_SERVER_DIR>` 换成本机内网 IP（对齐 Go `config.go:858`）。
///
/// 取不到内网 IP 时保留占位符并 warn，让用户手工填写，而不是生成一个空地址。
fn render_template() -> String {
    match gensokyo_core::local_ip() {
        Some(ip) => CONFIG_TEMPLATE.replace(SERVER_DIR_PLACEHOLDER, &ip.to_string()),
        None => {
            warn!("未能获取本机内网 IP，config.yml 中的 <YOUR_SERVER_DIR> 保留占位符，请手工填写");
            CONFIG_TEMPLATE.to_owned()
        }
    }
}

/// 同目录临时文件名：`config.yml` → `config.yml.tmp`。
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".tmp");
    path.with_file_name(name)
}

/// `create_new` 打开临时文件；上次异常退出留下的 tmp 先清掉再重试一次。
fn create_tmp(tmp: &Path) -> std::io::Result<fs::File> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)
    {
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(tmp)?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(tmp)
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!tmp_path(&path).exists(), "成功后不得残留 tmp");

        fs::write(&path, "version: 2\n").expect("写测试文件失败");
        assert!(
            !ensure_config_file(&path).expect("二次调用失败"),
            "已存在时不生成"
        );
        assert_eq!(fs::read_to_string(&path).expect("读回失败"), "version: 2\n");
    }

    #[test]
    fn stale_tmp_is_replaced() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        fs::write(tmp_path(&path), "半截").expect("写 tmp 失败");

        assert!(ensure_config_file(&path).expect("生成失败"));
        assert!(!tmp_path(&path).exists());
        assert!(
            fs::read_to_string(&path)
                .expect("读回失败")
                .starts_with("version: 1")
        );
    }
}
