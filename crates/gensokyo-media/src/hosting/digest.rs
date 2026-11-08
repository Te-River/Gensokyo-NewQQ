//! SHA-1 / HMAC-SHA1 / MD5：只为图床协议里写死的摘要。
//!
//! 为什么手写：腾讯云 COS 的 `q-sign-algorithm=sha1` 自签要 HMAC-SHA1，QQ频道图床的
//! CDN 地址要图片内容的 MD5（Go `imagehosting/qq_channel.go`），而本 crate 的依赖图里
//! 没有 `sha1` / `hmac` / `md-5`（`md-5` 只在 `gensokyo-storage`），这一轮也不允许扩边。
//! 三者都是公开规范的固定算法，按 RFC 3174 / RFC 2104 / RFC 1321 实现并带标准测试向量。
//!
//! 这里的摘要**不承担安全职责**：SHA-1 是 COS 协议规定的（服务端按同一算法校验），
//! MD5 只用来拼 CDN 路径，两者都不用于签名校验或口令。

/// SHA-1 摘要长度（字节）。
pub(crate) const SHA1_LEN: usize = 20;
/// HMAC / MD5 的分组长度（字节）。
const BLOCK_LEN: usize = 64;

/// 小写十六进制编码。
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
    out
}

/// SHA-1 十六进制摘要（小写），对齐 Go `sha1Hex` / `sha1HexN`。
pub(crate) fn sha1_hex(data: &[u8]) -> String {
    hex(&sha1(data))
}

/// HMAC-SHA1 十六进制摘要（小写），对齐 Go `hmacSha1` / `hmacSha1N`。
///
/// 注意 Go 侧把上一次 `hmacSha1` 的**十六进制字符串**再当 key 传给下一次（COS 的
/// `signKey`），这里不做任何解读，按字节入参即可复现。
pub(crate) fn hmac_sha1_hex(key: &[u8], message: &[u8]) -> String {
    let mut block = [0u8; BLOCK_LEN];
    if key.len() > BLOCK_LEN {
        block[..SHA1_LEN].copy_from_slice(&sha1(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }

    let mut inner = Vec::with_capacity(BLOCK_LEN + message.len());
    inner.extend(block.iter().map(|byte| byte ^ 0x36));
    inner.extend_from_slice(message);

    let inner_digest = sha1(&inner);
    let mut outer = Vec::with_capacity(BLOCK_LEN + SHA1_LEN);
    outer.extend(block.iter().map(|byte| byte ^ 0x5C));
    outer.extend_from_slice(&inner_digest);

    hex(&sha1(&outer))
}

/// MD5 十六进制摘要（大写），对齐 Go `strings.ToUpper(hex.EncodeToString(md5))`。
pub(crate) fn md5_hex_upper(data: &[u8]) -> String {
    hex(&md5(data)).to_uppercase()
}

/// SHA-1（RFC 3174）。
fn sha1(data: &[u8]) -> [u8; SHA1_LEN] {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];

    let mut message = data.to_vec();
    let bit_len = u64::try_from(data.len()).unwrap_or(u64::MAX / 8) * 8;
    message.push(0x80);
    while message.len() % BLOCK_LEN != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut schedule = [0u32; 80];
    for chunk in message.as_chunks::<BLOCK_LEN>().0 {
        for (word, bytes) in schedule[..16].iter_mut().zip(chunk.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*bytes);
        }
        for index in 16..80 {
            schedule[index] = (schedule[index - 3]
                ^ schedule[index - 8]
                ^ schedule[index - 14]
                ^ schedule[index - 16])
                .rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in schedule.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }

    let mut digest = [0u8; SHA1_LEN];
    for (word, bytes) in state.iter().zip(digest.as_chunks_mut::<4>().0) {
        *bytes = word.to_be_bytes();
    }
    digest
}

/// MD5 每轮的循环左移位数（RFC 1321 §3.4）。
const MD5_SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// MD5 每轮的加数 `K[i] = floor(2^32 × |sin(i+1)|)`（RFC 1321 §3.4）。
const MD5_CONSTANTS: [u32; 64] = [
    0xD76A_A478,
    0xE8C7_B756,
    0x2420_70DB,
    0xC1BD_CEEE,
    0xF57C_0FAF,
    0x4787_C62A,
    0xA830_4613,
    0xFD46_9501,
    0x6980_98D8,
    0x8B44_F7AF,
    0xFFFF_5BB1,
    0x895C_D7BE,
    0x6B90_1122,
    0xFD98_7193,
    0xA679_438E,
    0x49B4_0821,
    0xF61E_2562,
    0xC040_B340,
    0x265E_5A51,
    0xE9B6_C7AA,
    0xD62F_105D,
    0x0244_1453,
    0xD8A1_E681,
    0xE7D3_FBC8,
    0x21E1_CDE6,
    0xC337_07D6,
    0xF4D5_0D87,
    0x455A_14ED,
    0xA9E3_E905,
    0xFCEF_A3F8,
    0x676F_02D9,
    0x8D2A_4C8A,
    0xFFFA_3942,
    0x8771_F681,
    0x6D9D_6122,
    0xFDE5_380C,
    0xA4BE_EA44,
    0x4BDE_CFA9,
    0xF6BB_4B60,
    0xBEBF_BC70,
    0x289B_7EC6,
    0xEAA1_27FA,
    0xD4EF_3085,
    0x0488_1D05,
    0xD9D4_D039,
    0xE6DB_99E5,
    0x1FA2_7CF8,
    0xC4AC_5665,
    0xF429_2244,
    0x432A_FF97,
    0xAB94_23A7,
    0xFC93_A039,
    0x655B_59C3,
    0x8F0C_CC92,
    0xFFEF_F47D,
    0x8584_5DD1,
    0x6FA8_7E4F,
    0xFE2C_E6E0,
    0xA301_4314,
    0x4E08_11A1,
    0xF753_7E82,
    0xBD3A_F235,
    0x2AD7_D2BB,
    0xEB86_D391,
];

/// MD5（RFC 1321）。
fn md5(data: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476];

    let mut message = data.to_vec();
    let bit_len = u64::try_from(data.len()).unwrap_or(u64::MAX / 8) * 8;
    message.push(0x80);
    while message.len() % BLOCK_LEN != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in message.as_chunks::<BLOCK_LEN>().0 {
        let mut words = [0u32; 16];
        for (word, bytes) in words.iter_mut().zip(chunk.as_chunks::<4>().0) {
            *word = u32::from_le_bytes(*bytes);
        }

        let [mut a, mut b, mut c, mut d] = state;
        for index in 0..64 {
            let (f, slot) = match index {
                0..=15 => ((b & c) | (!b & d), index),
                16..=31 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let mixed = a
                .wrapping_add(f)
                .wrapping_add(MD5_CONSTANTS[index])
                .wrapping_add(words[slot]);
            let rotated = mixed.rotate_left(MD5_SHIFTS[index]);

            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    let mut digest = [0u8; 16];
    for (word, bytes) in state.iter().zip(digest.as_chunks_mut::<4>().0) {
        *bytes = word.to_le_bytes();
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::{hmac_sha1_hex, md5_hex_upper, sha1_hex};

    #[test]
    fn sha1_matches_rfc_3174_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // 跨分组边界：55 / 56 / 64 字节三种填充路径
        assert_eq!(sha1_hex(&[b'a'; 55]), sha1_hex(&[b'a'; 55]));
        assert_ne!(sha1_hex(&[b'a'; 55]), sha1_hex(&[b'a'; 56]));
        assert_eq!(sha1_hex(&[b'a'; 64]).len(), 40);
    }

    #[test]
    fn hmac_sha1_matches_rfc_2202_vector() {
        // RFC 2202 §3 测试用例 2
        assert_eq!(
            hmac_sha1_hex(b"Jefe", b"what do ya want for nothing?"),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
        // 测试用例 1（20 字节 key）
        assert_eq!(
            hmac_sha1_hex(&[0x0b; 20], b"Hi There"),
            "b617318655057264e28bc0b6fb378c8ef146be00"
        );
        // key 超过 64 字节时先做一次 SHA-1（测试用例 6）
        assert_eq!(
            hmac_sha1_hex(
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            ),
            "aa4ae5e15272d00e95705637ce8a3b55ed402112"
        );
    }

    #[test]
    fn md5_matches_rfc_1321_vectors() {
        assert_eq!(md5_hex_upper(b""), "D41D8CD98F00B204E9800998ECF8427E");
        assert_eq!(md5_hex_upper(b"abc"), "900150983CD24FB0D6963F7D28E17F72");
        assert_eq!(
            md5_hex_upper(b"message digest"),
            "F96B697D7CB7938D525A2F31AAF161D0"
        );
        assert_eq!(
            md5_hex_upper(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57EDF4A22BE3C955AC49DA2E2107B67A"
        );
    }
}
