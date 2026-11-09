//! 上传后内容审核：`t_audit`（腾讯云 COS CI）/ `a_audit`（阿里云内容安全）/ `b_audit=1`（百度 AIP）。
//!
//! 三家都是「先上传、再审核」，顺序与 Go 一致；审核不通过、或审核请求本身不可用，一律返回
//! 错误，调用方据此保留原引用——绝不把未审核的图片发布出去。
//!
//! # 请求形态（逐字段对齐 Go 所用 SDK 的调用点）
//!
//! - **腾讯云 COS**（Go `client.CI.ImageRecognition(ctx, objectKey, "")`）：
//!   `GET https://<bucket>.cos.<region>.myqcloud.com/<key>?ci-process=sensitive-content-recognition`，
//!   q-sign sha1 自签（`q-url-param-list=ci-process`、`q-header-list=host`）；响应是 XML
//!   `<RecognitionResult>`，`<Result>0</Result>` 为通过。
//! - **阿里云内容安全**（Go `green.CreateImageSyncScanRequest`）：
//!   `POST https://green.aliyuncs.com/green/image/scan`，ROA `acs` 签名（`Accept` / `Content-MD5` /
//!   `Content-Type` / `Date` + 排序后的 `x-acs-*`），body 为
//!   `{"scenes":["porn"],"tasks":[{"dataId":…,"url":"oss://<bucket>/<key>"}]}`；
//!   所有 `data[].results[].suggestion` 都是 `pass` 才算通过。
//! - **百度 AIP**（Go `clientBaidu.ImgCensorUrl(imageURL, {})`）：
//!   `POST https://aip.baidubce.com/rest/2.0/solution/v1/img_censor/v2/user_defined?aipSdk=golang`，
//!   云鉴权 `bce-auth-v1/<AK>/<date>/1800/host;x-bce-date/<sig>`，form `imgUrl=<URL>`；
//!   响应 `conclusion == "合规"` 才算通过。
//!
//! # 与 Go 的差异（都是「Go 会静默放行，这里报错」）
//! - 腾讯云：Go 在 `Result != 0` 时返回 `("", nil)`，调用方拿到空 URL 仍当成功；这里返回
//!   [`Error::AuditRejected`] 并带 `Result` 值。
//! - 阿里云：Go 只看 `suggestion`、忽略响应 `code`（596 = 内容安全未开通），`data` 为空即放行；
//!   这里 `code != 200` 直接报错。
//! - 阿里云：`oss://<bucket>/<key>` 的 bucket 用 `a_OSS_BucketName`（Go 用 `GetAppIDStr()`，
//!   那是一个不存在的桶）。
//! - 百度：`error_code != 0`（AK/SK 无效、URL 取不到等）在 Go 里只是 `conclusion` 不匹配，
//!   这里带上 `error_code` / `error_msg` 报错。

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::error::{Error, Result};
use crate::hosting::json;
use crate::hosting::qcloud;
use crate::hosting::transport::{HostingTransport, UploadBody, UploadMethod, UploadRequest};
use crate::hosting::unix_now;
use crate::hosting::{DEFAULT_UPLOAD_TIMEOUT, digest};
use crate::oss::{AliyunOssConfig, BaiduBosConfig, TencentCosConfig, host_of, iso8601};

/// COS 内容审核的查询串（Go SDK `ImageRecognition` 的 `CIProcess` 常量）。
const COS_CI_PROCESS: &str = "ci-process=sensitive-content-recognition";
/// 阿里云内容安全的 ROA 路径（Go SDK `ImageSyncScan` 的 `uriPattern`）。
const GREEN_PATH: &str = "/green/image/scan";
/// 阿里云内容安全的 API 版本。
const GREEN_VERSION: &str = "2018-05-09";
/// 百度 AIP 图片审核的 path 与查询串（Go SDK 的 `__imageCensorUserDefinedUrl` 与 `aipSdk` 参数）。
const BAIDU_CENSOR_PATH: &str = "/rest/2.0/solution/v1/img_censor/v2/user_defined";
const BAIDU_CENSOR_QUERY: &str = "aipSdk=golang";
/// 百度 AIP 的固定域名。
const BAIDU_AIP_HOST: &str = "aip.baidubce.com";
/// 内容类型：两家 POST 审核请求各自的 body 形态。
const GREEN_CONTENT_TYPE: &str = "application/octet-stream";
const BAIDU_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

/// 腾讯云 COS 内容审核：`GET /<key>?ci-process=sensitive-content-recognition`。
///
/// # Errors
/// 传输失败、非 2xx、响应里没有 `<Result>`；
/// [`Error::AuditRejected`] 表示 `Result != 0`（图片不合规）。
pub(crate) async fn tencent(
    transport: Arc<dyn HostingTransport>,
    config: &TencentCosConfig,
    key: &str,
) -> Result<()> {
    let (timestamp, _) = unix_now();
    let host = qcloud::object_host(&config.bucket, &config.region);
    let authorization = qcloud::signed(
        "get",
        &config.secret_id,
        &config.secret_key,
        key,
        &host,
        &[("ci-process", "sensitive-content-recognition")],
        timestamp,
    );
    let response = transport
        .send(UploadRequest {
            method: UploadMethod::Get,
            url: format!("https://{host}/{key}?{COS_CI_PROCESS}"),
            headers: vec![
                ("Host".to_owned(), host),
                ("Authorization".to_owned(), authorization),
            ],
            body: None,
            timeout: DEFAULT_UPLOAD_TIMEOUT,
        })
        .await?;
    ensure_success(response.status)?;
    let result = xml_int(&response.body, "Result").ok_or_else(|| {
        Error::HostingResponse(format!(
            "腾讯云 COS 审核响应缺少 <Result>: {}",
            snippet(&response.body)
        ))
    })?;
    if result != 0 {
        return Err(Error::AuditRejected(format!(
            "腾讯云 COS 内容审核未通过（Result={result}）"
        )));
    }
    Ok(())
}

/// 阿里云内容安全 `ImageSyncScan`：审核 OSS 里的对象。
///
/// # Errors
/// 端点里取不到 region（[`Error::HostingConfig`]）、传输失败、非 2xx、`code != 200`；
/// [`Error::AuditRejected`] 表示有 `suggestion != "pass"`。
pub(crate) async fn aliyun(
    transport: Arc<dyn HostingTransport>,
    config: &AliyunOssConfig,
    key: &str,
) -> Result<()> {
    let endpoint = green_endpoint(&config.endpoint)?;
    let (timestamp, nanos) = unix_now();
    let date = crate::oss::http_date(timestamp);
    // Go 用 uuid v4 当 dataId，这里只要求同一对象每次请求不同：md5(key) + 纳秒。
    let data_id = format!("{}-{nanos:x}", digest::md5_hex(key.as_bytes()));
    let body = format!(
        r#"{{"scenes":["porn"],"tasks":[{{"dataId":"{data_id}","url":"oss://{}/{key}"}}]}}"#,
        config.bucket
    );
    let content_md5 = BASE64.encode(digest::md5_digest(body.as_bytes()));
    let string_to_sign = format!(
        "POST\napplication/json\n{content_md5}\n{GREEN_CONTENT_TYPE}\n{date}\
         \nx-acs-action:ImageSyncScan\nx-acs-signature-method:HMAC-SHA1\
         \nx-acs-signature-version:1.0\nx-acs-version:{GREEN_VERSION}\n{GREEN_PATH}"
    );
    let signature = BASE64.encode(digest::hmac_sha1(
        config.access_key_secret.as_bytes(),
        string_to_sign.as_bytes(),
    ));
    let response = transport
        .send(UploadRequest {
            method: UploadMethod::Post,
            url: format!("https://{endpoint}{GREEN_PATH}"),
            headers: vec![
                ("Accept".to_owned(), "application/json".to_owned()),
                ("Content-MD5".to_owned(), content_md5),
                ("Date".to_owned(), date),
                ("x-acs-action".to_owned(), "ImageSyncScan".to_owned()),
                ("x-acs-signature-method".to_owned(), "HMAC-SHA1".to_owned()),
                ("x-acs-signature-version".to_owned(), "1.0".to_owned()),
                ("x-acs-version".to_owned(), GREEN_VERSION.to_owned()),
                (
                    "Authorization".to_owned(),
                    format!("acs {}:{signature}", config.access_key_id),
                ),
            ],
            body: Some(UploadBody {
                content_type: GREEN_CONTENT_TYPE.to_owned(),
                bytes: body.into_bytes(),
            }),
            timeout: DEFAULT_UPLOAD_TIMEOUT,
        })
        .await?;
    ensure_success(response.status)?;
    let value = json::parse(&response.body).ok_or_else(|| {
        Error::HostingResponse(format!(
            "阿里云内容审核响应不是 JSON: {}",
            snippet(&response.body)
        ))
    })?;
    if let Some(code) = value.path_i64(&["code"])
        && code != 200
    {
        return Err(Error::HostingResponse(format!(
            "阿里云内容审核请求失败: code={code}（596 表示内容安全未开通）"
        )));
    }
    for task in value.path_values(&["data"]) {
        for result in task.path_values(&["results"]) {
            let suggestion = result.path_str(&["suggestion"]).unwrap_or_default();
            if suggestion != "pass" {
                return Err(Error::AuditRejected(format!(
                    "阿里云内容审核未通过（suggestion={suggestion:?}）"
                )));
            }
        }
    }
    Ok(())
}

/// 百度 AIP 图片审核（`ImgCensorUrl`）：按公网 URL 审核 BOS 里刚上传的对象。
///
/// # Errors
/// 传输失败、非 2xx、响应不是 JSON、`error_code != 0`；
/// [`Error::AuditRejected`] 表示 `conclusion != "合规"`。
pub(crate) async fn baidu(
    transport: Arc<dyn HostingTransport>,
    config: &BaiduBosConfig,
    image_url: &str,
) -> Result<()> {
    let (timestamp, _) = unix_now();
    let date = iso8601(timestamp);
    let authorization = baidu_authorization(config, &date);
    let response = transport
        .send(UploadRequest {
            method: UploadMethod::Post,
            url: format!("https://{BAIDU_AIP_HOST}{BAIDU_CENSOR_PATH}?{BAIDU_CENSOR_QUERY}"),
            headers: vec![
                ("Host".to_owned(), BAIDU_AIP_HOST.to_owned()),
                ("x-bce-date".to_owned(), date),
                ("Authorization".to_owned(), authorization),
            ],
            body: Some(UploadBody {
                content_type: BAIDU_CONTENT_TYPE.to_owned(),
                bytes: format!("imgUrl={}", query_escape(image_url)).into_bytes(),
            }),
            timeout: DEFAULT_UPLOAD_TIMEOUT,
        })
        .await?;
    ensure_success(response.status)?;
    let value = json::parse(&response.body).ok_or_else(|| {
        Error::HostingResponse(format!(
            "百度 AIP 审核响应不是 JSON: {}",
            snippet(&response.body)
        ))
    })?;
    if let Some(code) = value.path_i64(&["error_code"])
        && code != 0
    {
        let message = value.path_str(&["error_msg"]).unwrap_or_default();
        return Err(Error::HostingResponse(format!(
            "百度 AIP 审核请求失败: error_code={code} error_msg={message:?}"
        )));
    }
    let conclusion = value.path_str(&["conclusion"]).unwrap_or_default();
    if conclusion != "合规" {
        return Err(Error::AuditRejected(format!(
            "百度 AIP 审核未通过（conclusion={conclusion:?}，仅「合规」放行）"
        )));
    }
    Ok(())
}

/// 百度云 `bce-auth-v1` 云鉴权头（Go `baseClient.Auth.setHeader`）。
///
/// 只签 `host;x-bce-date`，且 header 值先做一次 `QueryEscape`（Go SDK 原样照抄，
/// 所以签名里的日期是 `2026-11-09T10%3A00%3A00Z`）。
fn baidu_authorization(config: &BaiduBosConfig, date: &str) -> String {
    let canonical_headers = format!(
        "host:{}\nx-bce-date:{}",
        query_escape(BAIDU_AIP_HOST),
        query_escape(date)
    );
    let canonical_request =
        format!("POST\n{BAIDU_CENSOR_PATH}\n{BAIDU_CENSOR_QUERY}\n{canonical_headers}");
    let prefix = format!("bce-auth-v1/{}/{date}/1800", config.access_key);
    let signing_key = digest::hmac_sha256_hex(config.secret_key.as_bytes(), prefix.as_bytes());
    let signature = digest::hmac_sha256_hex(signing_key.as_bytes(), canonical_request.as_bytes());
    format!("{prefix}/host;x-bce-date/{signature}")
}

/// 阿里云内容安全的域名：region 从 `a_OSS_EndPoint` 里截（Go `config.GetRegionID` 的同一算法）。
///
/// 中国站各 region 在 Go SDK 的 endpoint 映射里都指向 `green.aliyuncs.com`；海外 region
/// 用 `green.<region>.aliyuncs.com`。endpoint 不是 `oss-<region>.aliyuncs.com` 这种标准形态
/// （自定义域名）时取不到 region，直接报配置错误。
fn green_endpoint(endpoint: &str) -> Result<String> {
    let region = region_of(endpoint).ok_or_else(|| {
        Error::HostingConfig(format!(
            "a_audit 需要从 a_OSS_EndPoint 解析 region（形如 oss-cn-hangzhou.aliyuncs.com），当前是 {endpoint:?}"
        ))
    })?;
    if region.starts_with("cn-") {
        Ok("green.aliyuncs.com".to_owned())
    } else {
        Ok(format!("green.{region}.aliyuncs.com"))
    }
}

/// Go `config.GetRegionID`：`oss-cn-hangzhou.aliyuncs.com` → `cn-hangzhou`。
fn region_of(endpoint: &str) -> Option<String> {
    let host = host_of(endpoint);
    let first = host.split('.').next()?;
    let parts: Vec<&str> = first.splitn(3, '-').collect();
    if parts.len() < 3 {
        return None;
    }
    Some(format!("{}-{}", parts[1], parts[2]))
}

/// Go `url.QueryEscape`：`A-Z a-z 0-9 - _ . ~` 原样，空格变 `+`，其余 `%XX`（大写十六进制）。
fn query_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            byte => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// 从 XML 响应里取一个整数字段（COS CI 只用到 `<Result>`）。
fn xml_int(body: &[u8], tag: &str) -> Option<i64> {
    let text = std::str::from_utf8(body).ok()?;
    let open = format!("<{tag}>");
    let start = text.find(&open)? + open.len();
    let end = start + text[start..].find(&format!("</{tag}>"))?;
    text[start..end].trim().parse().ok()
}

/// 非 2xx 一律失败（三家 SDK 同口径）。
fn ensure_success(status: u16) -> Result<()> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(Error::HttpStatus(status))
    }
}

/// 响应体摘要（错误信息用）：最多 120 个字符，避免把整段 XML / JSON 写进日志。
fn snippet(body: &[u8]) -> String {
    String::from_utf8_lossy(body).chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::{green_endpoint, query_escape, region_of, xml_int};

    #[test]
    fn reads_region_from_standard_endpoints() {
        assert_eq!(
            region_of("https://oss-cn-hangzhou.aliyuncs.com").as_deref(),
            Some("cn-hangzhou")
        );
        assert_eq!(
            region_of("oss-cn-beijing.aliyuncs.com/").as_deref(),
            Some("cn-beijing")
        );
        assert_eq!(
            region_of("oss-cn-hangzhou-internal.aliyuncs.com").as_deref(),
            Some("cn-hangzhou-internal"),
            "与 Go SplitN(…, 3) 一致"
        );
        assert_eq!(region_of("cdn.example.com"), None);
        assert_eq!(region_of(""), None);
    }

    #[test]
    fn maps_regions_to_green_endpoints() {
        assert_eq!(
            green_endpoint("oss-cn-hangzhou.aliyuncs.com").expect("cn 区域"),
            "green.aliyuncs.com"
        );
        assert_eq!(
            green_endpoint("oss-ap-southeast-1.aliyuncs.com").expect("海外区域"),
            "green.ap-southeast-1.aliyuncs.com"
        );
        assert!(
            green_endpoint("cdn.example.com").is_err(),
            "自定义域名取不到 region"
        );
    }

    #[test]
    fn escapes_like_go_query_escape() {
        assert_eq!(
            query_escape("https://aip.baidubce.com/x?y=1"),
            "https%3A%2F%2Faip.baidubce.com%2Fx%3Fy%3D1"
        );
        assert_eq!(
            query_escape("2026-11-09T10:00:00Z"),
            "2026-11-09T10%3A00%3A00Z"
        );
        assert_eq!(query_escape("a b~c_d-e.f"), "a+b~c_d-e.f");
    }

    #[test]
    fn extracts_result_from_cos_xml() {
        let body = br#"<?xml version="1.0"?><RecognitionResult><JobId>j</JobId><Result>2</Result></RecognitionResult>"#;
        assert_eq!(xml_int(body, "Result"), Some(2));
        assert_eq!(
            xml_int(b"<RecognitionResult><Result>0</Result>", "Result"),
            Some(0)
        );
        assert_eq!(
            xml_int(b"<Response><Code>NoSuchKey</Code></Response>", "Result"),
            None
        );
        assert_eq!(xml_int(b"", "Result"), None);
    }
}
