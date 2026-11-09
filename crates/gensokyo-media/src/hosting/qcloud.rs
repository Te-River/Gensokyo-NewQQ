//! 腾讯云 COS 的 `q-sign-algorithm=sha1` 自签，供 oss_type 4（[`super::cos`]）、
//! oss_type 10（[`super::nature`]）与内置云 OSS 的内容审核（[`crate::oss`]）共用。
//!
//! Go 侧在 `cos.go` 与 `nature.go` 里各抄了一份签名代码（`hmacSha1` / `hmacSha1N`），
//! 逐字节等价；这里合成一份，避免两处漂移。签名串与字段顺序照 Go：
//!
//! ```text
//! signTime = "<ts>;<ts+3600>"
//! signKey  = hex(hmac_sha1(secret_key, signTime))
//! fmtStr   = "<method>\n/<key>\n<url params>\nhost=<host>\n"
//! sts      = "sha1\n<signTime>\n<hex(sha1(fmtStr))>\n"
//! sig      = hex(hmac_sha1(signKey, sts))       // signKey 是十六进制字符串本身
//! ```

use crate::hosting::digest;

/// 签名有效期（秒），对齐 Go 的 `ts+3600`。
pub const SIGN_TTL_SECONDS: i64 = 3600;

/// 对象的 COS 域名（虚拟主机风格），对齐 Go `fmt.Sprintf("%s.cos.%s.myqcloud.com", bucket, region)`。
pub(crate) fn object_host(bucket: &str, region: &str) -> String {
    format!("{bucket}.cos.{region}.myqcloud.com")
}

/// `q-sign-time` / `q-key-time` 的取值。
fn sign_time(timestamp: i64) -> String {
    format!("{timestamp};{}", timestamp + SIGN_TTL_SECONDS)
}

/// 上传（`PUT`，无查询串）用的 `Authorization` 头。
pub(crate) fn authorization(
    secret_id: &str,
    secret_key: &str,
    key: &str,
    host: &str,
    timestamp: i64,
) -> String {
    signed("put", secret_id, secret_key, key, host, &[], timestamp)
}

/// 生成 `Authorization` 头：`method` 小写；`query` 按 key 排序后同时进
/// `q-url-param-list` 与签名串（COS 内容审核的 `GET …?ci-process=…` 需要）。
pub(crate) fn signed(
    method: &str,
    secret_id: &str,
    secret_key: &str,
    key: &str,
    host: &str,
    query: &[(&str, &str)],
    timestamp: i64,
) -> String {
    let mut params = query.to_vec();
    params.sort_by(|left, right| left.0.cmp(right.0));
    let url_params = params
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let param_list = params
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(";");
    let sign_time = sign_time(timestamp);
    let sign_key = digest::hmac_sha1_hex(secret_key.as_bytes(), sign_time.as_bytes());
    let format_string = format!("{method}\n/{key}\n{url_params}\nhost={host}\n");
    let string_to_sign = format!(
        "sha1\n{sign_time}\n{}\n",
        digest::sha1_hex(format_string.as_bytes())
    );
    let signature = digest::hmac_sha1_hex(sign_key.as_bytes(), string_to_sign.as_bytes());
    format!(
        "q-sign-algorithm=sha1&q-ak={secret_id}&q-sign-time={sign_time}&q-key-time={sign_time}\
         &q-header-list=host&q-url-param-list={param_list}&q-signature={signature}"
    )
}

#[cfg(test)]
mod tests {
    use super::{SIGN_TTL_SECONDS, authorization, object_host, signed};

    #[test]
    fn authorization_matches_cos_qsign_layout() {
        // 期望值用独立实现（.NET HMACSHA1 / SHA1）按 Go `tryCOS` 的同一套字符串算出：
        // 若这里与 Go 的 q-sign 不一致，说明签名串的字段或分隔符被改动了。
        assert_eq!(
            authorization(
                "AKIDtest",
                "secret-key",
                "gensokyo/1700000000/a.png",
                "bucket-125.cos.ap-guangzhou.myqcloud.com",
                1_700_000_000
            ),
            "q-sign-algorithm=sha1&q-ak=AKIDtest&q-sign-time=1700000000;1700003600\
             &q-key-time=1700000000;1700003600&q-header-list=host&q-url-param-list=\
             &q-signature=f72de69aa6a800d83ba449a9e2430c6c55dbe092"
        );
    }

    #[test]
    fn authorization_binds_key_host_and_secret() {
        let base = authorization("id", "secret", "k/a.png", "b.cos.r.myqcloud.com", 10);
        for other in [
            authorization("id", "secret", "k/b.png", "b.cos.r.myqcloud.com", 10),
            authorization("id", "secret", "k/a.png", "other.cos.r.myqcloud.com", 10),
            authorization("id", "other-secret", "k/a.png", "b.cos.r.myqcloud.com", 10),
            authorization("id", "secret", "k/a.png", "b.cos.r.myqcloud.com", 11),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn query_signing_matches_get_with_url_params() {
        // COS 内容审核：GET + `ci-process` 同时进 q-url-param-list 与签名串
        let signed_header = signed(
            "get",
            "AKIDtest",
            "secret-key",
            "1700000000.jpg",
            "bucket-125.cos.ap-shanghai.myqcloud.com",
            &[("ci-process", "sensitive-content-recognition")],
            1_700_000_000,
        );
        assert!(
            signed_header.contains("q-url-param-list=ci-process&q-signature="),
            "{signed_header}"
        );
        assert_ne!(
            signed_header,
            signed(
                "get",
                "AKIDtest",
                "secret-key",
                "1700000000.jpg",
                "bucket-125.cos.ap-shanghai.myqcloud.com",
                &[],
                1_700_000_000,
            ),
            "查询串参与签名"
        );
        assert_ne!(
            signed_header,
            authorization(
                "AKIDtest",
                "secret-key",
                "1700000000.jpg",
                "bucket-125.cos.ap-shanghai.myqcloud.com",
                1_700_000_000,
            ),
            "方法参与签名（put ≠ get）"
        );
    }

    #[test]
    fn hosts_and_ttl_follow_go() {
        assert_eq!(
            object_host("sgame-data-service-1252931805", "ap-nanjing"),
            "sgame-data-service-1252931805.cos.ap-nanjing.myqcloud.com"
        );
        assert_eq!(SIGN_TTL_SECONDS, 3600);
    }
}
