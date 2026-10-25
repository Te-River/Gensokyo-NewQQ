//! silk 语音编解码：调用外部 `ffmpeg` + `pts` 进程。
//!
//! # M2d 范围
//! - [`SilkCodec::encode_to_silk`]：任意音频 → 16-bit 单声道 PCM（`ffmpeg`）→ Silk v3（`pts`）。
//! - [`SilkCodec::decode_silk`]：Silk v3 → PCM（`pts`）→ 目标容器 / 编码（`ffmpeg`，按输出扩展名推断）。
//! - [`ProcessRunner`]：唯一的进程出口。生产实现 [`CommandRunner`]（`std::process::Command`），
//!   测试用假执行器断言程序名 / 参数顺序 / 错误映射，全程不启动真实进程。
//!
//! 参数顺序对齐 Go `silk/silk.go` 的 `encode`（**不加** `-y`，临时 PCM 路径由唯一名保证是全新文件）：
//!
//! ```text
//! ffmpeg -i <input> -f s16le -ar <rate> -ac 1 <pcm>
//! pts    -i <pcm>   -o <output> -s <rate>
//! ```
//!
//! 解码是同一对工具的反向组合：
//!
//! ```text
//! pts    -i <input> -o <pcm> -s <rate>
//! ffmpeg -f s16le -ar <rate> -ac 1 -i <pcm> <output>
//! ```
//!
//! # 平台与二进制选择（M2d 不嵌入）
//! Go 通过 `go:embed exec/*` 把 8 个平台的 `silk_codec-*` 打进二进制，运行时解到临时文件、
//! chmod 0700 再执行。本 crate **不嵌入** `silk/exec/`（体积与平台矩阵留到 M4），改为依赖运行
//! 环境已安装的外部工具：
//!
//! | 角色 | 默认程序 | 覆盖方式 |
//! |------|----------|----------|
//! | PCM 转换 / 重采样 | `ffmpeg`（PATH） | `GENSOKYO_FFMPEG=<路径或程序名>` |
//! | Silk 编解码 | `pts`（PATH） | `GENSOKYO_PTS=<路径或程序名>` |
//!
//! 空字符串 / 纯空白视为未设置，回退默认名。程序找不到时返回 [`Error::SilkUnavailable`]，
//! 不 panic；非零退出、产物缺失或参数非法返回 [`Error::SilkCodec`]。
//!
//! # 与 Go 的差异
//! - **二进制来源**：Go 内嵌 8 个平台产物；这里只调用外部 `ffmpeg` / `pts`（部署要求见上表）。
//! - **内建快速路径**：Go 对合格 WAV 直接取 PCM，并用内置 MP3 解码器 + 线性重采样 + 立体声下混
//!   （`wavToPcm` / `mp3ToPcm` / `resamplePcm` / `stereoToMono`）；本模块统一走 `ffmpeg`
//!   （`-ar` / `-ac 1` 已覆盖重采样与下混），没有内建解码器分支。
//! - **失败语义**：Go 的 ffmpeg / silk_codec 失败只 `mylog` 后返回 `nil`（调用方拿到空音频）；
//!   这里返回 [`Error::SilkUnavailable`] / [`Error::SilkCodec`]，由调用方决定降级。
//! - **产物位置**：Go 把 silk 写到 `os.TempDir()/gensokyo-silk` 再读回内存；这里 `pts` 直接写
//!   调用方给的 `output`，不回读（少一次拷贝，调用方自行读文件）。
//! - **临时 PCM**：`<os temp>/gensokyo-silk/<pid>-<seq>.pcm`，成功与失败都由 `Drop` 清理（对齐 Go 的
//!   应用私有临时目录）；清理失败只 `tracing::warn!` 留线索，不掩盖主流程结果。
//! - **比特率**：Go 读 `record_bitRate` 但没有传给 `pts`；这里同样只暴露采样率。
//! - **阻塞**：整条链路是同步 `std::process`；在异步运行时里请放进 `spawn_blocking`。
//! - **不校验内容**：Go 另有 `IsAMRorSILK` 头判断；本模块不做嗅探，交给 `pts` 报错。
//!
//! # 部署要求
//! 1. `ffmpeg` 在 PATH 中（或设 `GENSOKYO_FFMPEG`）。
//! 2. Silk v3 编解码器 `pts` 在 PATH 中（或设 `GENSOKYO_PTS`）；Windows 下需 `.exe` 或完整路径。
//! 3. 进程对系统临时目录可写（PCM 中转）。
//! 4. 不随二进制分发 `silk/exec/`；容器镜像需显式安装上述工具。

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};

/// QQ 语音默认采样率，对齐 Go `template/config.yml` 的 `record_sampleRate: 24000`。
pub const DEFAULT_SAMPLE_RATE: u32 = 24_000;

/// 覆盖 `ffmpeg` 程序名 / 路径的环境变量。
pub const FFMPEG_ENV: &str = "GENSOKYO_FFMPEG";
/// 覆盖 `pts` 程序名 / 路径的环境变量。
pub const PTS_ENV: &str = "GENSOKYO_PTS";
/// 默认 `ffmpeg` 程序名（走 PATH 查找）。
pub const DEFAULT_FFMPEG: &str = "ffmpeg";
/// 默认 `pts` 程序名（走 PATH 查找）。
pub const DEFAULT_PTS: &str = "pts";

/// 临时 PCM 目录名（对齐 Go `os.TempDir()/gensokyo-silk`）。
const TEMP_DIR: &str = "gensokyo-silk";
/// 错误信息里保留的 stderr 字符上限，避免把整段 ffmpeg 日志塞进错误链。
const STDERR_LIMIT: usize = 400;

/// 临时 PCM 文件名的进程内序号。
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 外部进程的执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    /// 退出码；被信号终止等无退出码的情况为 `None`。
    pub status: Option<i32>,
    /// 标准输出（未使用，保留给诊断）。
    pub stdout: Vec<u8>,
    /// 标准错误，非零退出时拼进错误信息。
    pub stderr: Vec<u8>,
}

impl ProcessOutput {
    /// 是否以退出码 0 正常结束。
    #[must_use]
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// 去掉首尾空白的 stderr（有损 UTF-8，超长截断）。
    #[must_use]
    pub fn stderr_lossy(&self) -> String {
        let text = String::from_utf8_lossy(&self.stderr);
        let trimmed = text.trim();
        if trimmed.chars().count() <= STDERR_LIMIT {
            return trimmed.to_owned();
        }
        let head: String = trimmed.chars().take(STDERR_LIMIT).collect();
        format!("{head}…")
    }
}

/// 进程出口：把「跑一个外部程序」抽象成可注入依赖。
pub trait ProcessRunner: Send + Sync {
    /// 运行 `program`，参数按 `args` 原样传入。
    ///
    /// 实现只负责启动进程并收集输出；找不到程序时返回 [`Error::Io`]（`NotFound`），
    /// 由 [`SilkCodec`] 统一映射成 [`Error::SilkUnavailable`]。
    fn run(&self, program: &str, args: &[String]) -> Result<ProcessOutput>;
}

impl<T: ProcessRunner + ?Sized> ProcessRunner for Arc<T> {
    fn run(&self, program: &str, args: &[String]) -> Result<ProcessOutput> {
        (**self).run(program, args)
    }
}

/// 生产实现：`std::process::Command`，捕获 stdout / stderr。
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandRunner;

impl ProcessRunner for CommandRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<ProcessOutput> {
        // Command::output 会等待子进程结束并收集输出；启动失败（含 NotFound）经 #[from] 进 Error::Io
        let output = std::process::Command::new(program).args(args).output()?;
        Ok(ProcessOutput {
            status: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// silk 编解码器：外部 `ffmpeg` + `pts`，进程出口可注入。
#[derive(Clone)]
pub struct SilkCodec {
    runner: Arc<dyn ProcessRunner>,
    ffmpeg: String,
    pts: String,
}

impl fmt::Debug for SilkCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SilkCodec")
            .field("ffmpeg", &self.ffmpeg)
            .field("pts", &self.pts)
            .finish_non_exhaustive()
    }
}

impl Default for SilkCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl SilkCodec {
    /// 生产构造：程序名从环境变量解析（`GENSOKYO_FFMPEG` / `GENSOKYO_PTS`，缺省 `ffmpeg` / `pts`）。
    #[must_use]
    pub fn new() -> Self {
        Self::with_programs(
            program_from_env(FFMPEG_ENV, DEFAULT_FFMPEG),
            program_from_env(PTS_ENV, DEFAULT_PTS),
        )
    }

    /// 指定程序名 / 路径，进程出口仍是 [`CommandRunner`]。
    pub fn with_programs(ffmpeg: impl Into<String>, pts: impl Into<String>) -> Self {
        Self::with_runner_and_programs(CommandRunner, ffmpeg, pts)
    }

    /// 注入进程出口，程序名用默认值（测试主入口）。
    pub fn with_runner(runner: impl ProcessRunner + 'static) -> Self {
        Self::with_runner_and_programs(runner, DEFAULT_FFMPEG, DEFAULT_PTS)
    }

    /// 注入进程出口并指定程序名 / 路径。
    pub fn with_runner_and_programs(
        runner: impl ProcessRunner + 'static,
        ffmpeg: impl Into<String>,
        pts: impl Into<String>,
    ) -> Self {
        Self {
            runner: Arc::new(runner),
            ffmpeg: ffmpeg.into(),
            pts: pts.into(),
        }
    }

    /// 当前生效的 `ffmpeg` 程序名 / 路径。
    #[must_use]
    pub fn ffmpeg_program(&self) -> &str {
        &self.ffmpeg
    }

    /// 当前生效的 `pts` 程序名 / 路径。
    #[must_use]
    pub fn pts_program(&self) -> &str {
        &self.pts
    }

    /// 把 `input` 编码成 `output` 处的 Silk v3，`sample_rate` 同时用于 ffmpeg 重采样与 `pts`。
    ///
    /// 中间的 16-bit 单声道 PCM 落在系统临时目录，返回（无论成功或失败）前删除。
    pub fn encode_to_silk(&self, input: &Path, output: &Path, sample_rate: u32) -> Result<()> {
        validate_sample_rate(sample_rate)?;
        let pcm = TempPcm::new()?;
        let rate = sample_rate.to_string();

        let ffmpeg_args = vec![
            "-i".to_owned(),
            path_arg(input)?,
            "-f".to_owned(),
            "s16le".to_owned(),
            "-ar".to_owned(),
            rate.clone(),
            "-ac".to_owned(),
            "1".to_owned(),
            path_arg(pcm.path())?,
        ];
        self.run_checked(&self.ffmpeg, &ffmpeg_args)?;

        let pts_args = vec![
            "-i".to_owned(),
            path_arg(pcm.path())?,
            "-o".to_owned(),
            path_arg(output)?,
            "-s".to_owned(),
            rate,
        ];
        self.run_checked(&self.pts, &pts_args)?;

        ensure_output(output, &self.pts)
    }

    /// 把 `input` 的 Silk v3 解码成 `output`（容器 / 编码由 ffmpeg 按输出扩展名推断）。
    ///
    /// `pts` 以 [`DEFAULT_SAMPLE_RATE`] 解出 16-bit 单声道 PCM，`ffmpeg` 再按同一采样率读取，
    /// 不做二次重采样。中途的 PCM 文件同样在返回前删除。
    pub fn decode_silk(&self, input: &Path, output: &Path) -> Result<()> {
        let pcm = TempPcm::new()?;
        let rate = DEFAULT_SAMPLE_RATE.to_string();

        let pts_args = vec![
            "-i".to_owned(),
            path_arg(input)?,
            "-o".to_owned(),
            path_arg(pcm.path())?,
            "-s".to_owned(),
            rate.clone(),
        ];
        self.run_checked(&self.pts, &pts_args)?;

        let ffmpeg_args = vec![
            "-f".to_owned(),
            "s16le".to_owned(),
            "-ar".to_owned(),
            rate,
            "-ac".to_owned(),
            "1".to_owned(),
            "-i".to_owned(),
            path_arg(pcm.path())?,
            path_arg(output)?,
        ];
        self.run_checked(&self.ffmpeg, &ffmpeg_args)?;

        ensure_output(output, &self.ffmpeg)
    }

    /// 执行一次进程并把「找不到程序 / 非零退出」翻译成 crate 错误。
    fn run_checked(&self, program: &str, args: &[String]) -> Result<ProcessOutput> {
        let output = match self.runner.run(program, args) {
            Ok(output) => output,
            Err(Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::SilkUnavailable(format!(
                    "未找到外部程序 `{program}`（{err}）；silk 编解码依赖 ffmpeg 与 pts，\
                     请安装或用 {FFMPEG_ENV} / {PTS_ENV} 指定路径"
                )));
            }
            Err(err) => return Err(err),
        };
        if output.success() {
            return Ok(output);
        }
        Err(Error::SilkCodec(format!(
            "`{program}` 执行失败（退出码 {:?}）: {}",
            output.status,
            output.stderr_lossy()
        )))
    }
}

/// 采样率 0 会让 ffmpeg / pts 直接报参数错误，提前拒绝。
fn validate_sample_rate(sample_rate: u32) -> Result<()> {
    if sample_rate == 0 {
        return Err(Error::SilkCodec("采样率必须大于 0".to_owned()));
    }
    Ok(())
}

/// 路径 → 参数。非 UTF-8 路径无法无损传给外部进程，直接报错而不是静默替换字符。
fn path_arg(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        Error::SilkCodec(format!(
            "路径不是合法 UTF-8，无法传给外部进程: {}",
            path.display()
        ))
    })
}

/// `program` 声称成功后确认产物存在且非空。
fn ensure_output(path: &Path, program: &str) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .map_err(|err| Error::SilkCodec(format!("`{program}` 未产出 {}: {err}", path.display())))?;
    if metadata.len() == 0 {
        return Err(Error::SilkCodec(format!(
            "`{program}` 产出为空文件: {}",
            path.display()
        )));
    }
    Ok(())
}

/// 环境变量解析：缺失或空白回退默认程序名。
fn program_from_env(var: &str, default: &str) -> String {
    resolve_program(var, default, |key| std::env::var(key).ok())
}

/// [`program_from_env`] 的纯函数内核，便于测试不触碰进程环境。
fn resolve_program<F>(var: &str, default: &str, lookup: F) -> String
where
    F: FnOnce(&str) -> Option<String>,
{
    match lookup(var) {
        Some(value) if !value.trim().is_empty() => value,
        _ => default.to_owned(),
    }
}

/// 自动清理的临时 PCM 路径。
struct TempPcm {
    path: PathBuf,
}

impl TempPcm {
    fn new() -> Result<Self> {
        let dir = std::env::temp_dir().join(TEMP_DIR);
        std::fs::create_dir_all(&dir).map_err(|err| {
            Error::SilkCodec(format!("创建 silk 临时目录 {} 失败: {err}", dir.display()))
        })?;
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            path: dir.join(format!("{}-{seq}.pcm", std::process::id())),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPcm {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_file(&self.path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.path.display(),
                %err,
                "silk 临时 PCM 清理失败"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        DEFAULT_SAMPLE_RATE, PTS_ENV, ProcessOutput, ProcessRunner, SilkCodec, resolve_program,
    };
    use crate::error::{Error, Result};

    /// 一次被记录的进程调用。
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Invocation {
        program: String,
        args: Vec<String>,
    }

    /// 假执行器的行为：可按程序名写文件、返回错误或失败输出。
    type Responder = Box<dyn Fn(&Invocation) -> Result<ProcessOutput> + Send + Sync>;

    /// 假执行器：记录每次调用，行为由闭包决定（可写文件、可返回错误）。
    struct FakeRunner {
        calls: Mutex<Vec<Invocation>>,
        respond: Responder,
    }

    impl FakeRunner {
        fn new(
            respond: impl Fn(&Invocation) -> Result<ProcessOutput> + Send + Sync + 'static,
        ) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                respond: Box::new(respond),
            }
        }

        fn calls(&self) -> Vec<Invocation> {
            self.calls.lock().expect("calls 锁未中毒").clone()
        }
    }

    impl ProcessRunner for FakeRunner {
        fn run(&self, program: &str, args: &[String]) -> Result<ProcessOutput> {
            let invocation = Invocation {
                program: program.to_owned(),
                args: args.to_vec(),
            };
            self.calls
                .lock()
                .expect("calls 锁未中毒")
                .push(invocation.clone());
            (self.respond)(&invocation)
        }
    }

    /// 成功的进程输出。
    fn ok() -> ProcessOutput {
        ProcessOutput {
            status: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    /// 非零退出。
    fn failed(status: i32, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            status: Some(status),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    /// 程序缺失。
    fn not_found() -> Error {
        Error::Io(io::Error::from(io::ErrorKind::NotFound))
    }

    /// 每个测试独占的临时目录，析构时整目录删除。
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("时钟早于 epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gensokyo-silk-test-{label}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("创建测试目录");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn file(&self, name: &str, contents: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("写测试文件");
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 记录 ffmpeg 写出的 PCM 路径，供调用方断言清理。
    type PcmSlot = Arc<Mutex<Option<PathBuf>>>;

    #[test]
    fn encode_runs_ffmpeg_then_pts_and_removes_pcm() {
        let dir = TestDir::new("encode");
        let input = dir.file("input.mp3", b"ID3\x04");
        let output = dir.path().join("out.silk");
        let output_for_fake = output.clone();
        let pcm_seen: PcmSlot = Arc::new(Mutex::new(None));

        let runner = Arc::new(FakeRunner::new({
            let pcm_seen = Arc::clone(&pcm_seen);
            move |inv| match inv.program.as_str() {
                "ffmpeg" => {
                    let pcm = PathBuf::from(inv.args.last().expect("ffmpeg 末参是 PCM 路径"));
                    std::fs::write(&pcm, b"pcm-bytes").expect("写 PCM");
                    *pcm_seen.lock().expect("pcm 锁未中毒") = Some(pcm);
                    Ok(ok())
                }
                "pts" => {
                    std::fs::write(&output_for_fake, b"\x02#!SILK_V3").expect("写 silk");
                    Ok(ok())
                }
                other => panic!("意外的程序: {other}"),
            }
        }));
        let codec = SilkCodec::with_runner(Arc::clone(&runner));

        codec
            .encode_to_silk(&input, &output, 24_000)
            .expect("编码成功");

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "应恰好调用 ffmpeg + pts: {calls:?}");

        assert_eq!(calls[0].program, "ffmpeg");
        assert_eq!(
            calls[0].args,
            [
                "-i",
                input.to_str().expect("UTF-8 路径"),
                "-f",
                "s16le",
                "-ar",
                "24000",
                "-ac",
                "1",
                calls[0].args[8].as_str(),
            ]
        );
        assert!(
            calls[0].args[8].ends_with(".pcm"),
            "PCM 应是临时文件: {}",
            calls[0].args[8]
        );

        assert_eq!(calls[1].program, "pts");
        assert_eq!(
            calls[1].args,
            [
                "-i",
                calls[0].args[8].as_str(),
                "-o",
                output.to_str().expect("UTF-8 路径"),
                "-s",
                "24000",
            ]
        );

        let pcm = pcm_seen
            .lock()
            .expect("pcm 锁未中毒")
            .clone()
            .expect("ffmpeg 收到 PCM 路径");
        assert!(!pcm.exists(), "临时 PCM 未清理: {}", pcm.display());
        assert_eq!(std::fs::read(&output).expect("读取 silk"), b"\x02#!SILK_V3");
    }

    #[test]
    fn decode_runs_pts_then_ffmpeg_and_removes_pcm() {
        let dir = TestDir::new("decode");
        let input = dir.file("in.silk", b"\x02#!SILK_V3");
        let output = dir.path().join("out.wav");
        let output_for_fake = output.clone();
        let pcm_seen: PcmSlot = Arc::new(Mutex::new(None));
        let rate = DEFAULT_SAMPLE_RATE.to_string();

        let runner = Arc::new(FakeRunner::new({
            let rate = rate.clone();
            let pcm_seen = Arc::clone(&pcm_seen);
            move |inv| match inv.program.as_str() {
                "pts" => {
                    assert_eq!(inv.args[5], rate, "pts 采样率");
                    let pcm = PathBuf::from(&inv.args[3]);
                    std::fs::write(&pcm, b"pcm-bytes").expect("写 PCM");
                    *pcm_seen.lock().expect("pcm 锁未中毒") = Some(pcm);
                    Ok(ok())
                }
                "ffmpeg" => {
                    std::fs::write(&output_for_fake, b"RIFF\x00\x00\x00\x00WAVE").expect("写 wav");
                    Ok(ok())
                }
                other => panic!("意外的程序: {other}"),
            }
        }));
        let codec = SilkCodec::with_runner(Arc::clone(&runner));

        codec.decode_silk(&input, &output).expect("解码成功");

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "应恰好调用 pts + ffmpeg: {calls:?}");

        assert_eq!(calls[0].program, "pts");
        assert_eq!(
            calls[0].args,
            [
                "-i",
                input.to_str().expect("UTF-8 路径"),
                "-o",
                calls[0].args[3].as_str(),
                "-s",
                rate.as_str(),
            ]
        );

        assert_eq!(calls[1].program, "ffmpeg");
        assert_eq!(
            calls[1].args,
            [
                "-f",
                "s16le",
                "-ar",
                rate.as_str(),
                "-ac",
                "1",
                "-i",
                calls[0].args[3].as_str(),
                output.to_str().expect("UTF-8 路径"),
            ]
        );

        let pcm = pcm_seen
            .lock()
            .expect("pcm 锁未中毒")
            .clone()
            .expect("pts 收到 PCM 路径");
        assert!(!pcm.exists(), "临时 PCM 未清理: {}", pcm.display());
    }

    #[test]
    fn missing_ffmpeg_maps_to_silk_unavailable() {
        let dir = TestDir::new("missing-ffmpeg");
        let input = dir.file("input.mp3", b"ID3");
        let output = dir.path().join("out.silk");

        let runner = Arc::new(FakeRunner::new(|_| Err(not_found())));
        let codec = SilkCodec::with_runner(Arc::clone(&runner));

        let err = codec
            .encode_to_silk(&input, &output, 24_000)
            .expect_err("ffmpeg 缺失必须报错");
        match err {
            Error::SilkUnavailable(message) => {
                assert!(message.contains("ffmpeg"), "错误应点明程序: {message}");
            }
            other => panic!("错误映射不对: {other:?}"),
        }
        assert_eq!(runner.calls().len(), 1, "ffmpeg 失败后不应调用 pts");
    }

    #[test]
    fn missing_pts_maps_to_silk_unavailable() {
        let dir = TestDir::new("missing-pts");
        let input = dir.file("input.mp3", b"ID3");
        let output = dir.path().join("out.silk");
        let pcm_seen: PcmSlot = Arc::new(Mutex::new(None));

        let runner = Arc::new(FakeRunner::new({
            let pcm_seen = Arc::clone(&pcm_seen);
            move |inv| {
                if inv.program == "ffmpeg" {
                    let pcm = PathBuf::from(inv.args.last().expect("ffmpeg 末参是 PCM 路径"));
                    std::fs::write(&pcm, b"pcm-bytes").expect("写 PCM");
                    *pcm_seen.lock().expect("pcm 锁未中毒") = Some(pcm);
                    Ok(ok())
                } else {
                    Err(not_found())
                }
            }
        }));
        let codec = SilkCodec::with_runner(Arc::clone(&runner));

        let err = codec
            .encode_to_silk(&input, &output, 24_000)
            .expect_err("pts 缺失必须报错");
        match err {
            Error::SilkUnavailable(message) => {
                assert!(message.contains("pts"), "错误应点明程序: {message}");
            }
            other => panic!("错误映射不对: {other:?}"),
        }
        assert_eq!(runner.calls().len(), 2, "ffmpeg 先成功，随后 pts 失败");

        let pcm = pcm_seen
            .lock()
            .expect("pcm 锁未中毒")
            .clone()
            .expect("ffmpeg 收到 PCM 路径");
        assert!(!pcm.exists(), "失败路径也要清理 PCM: {}", pcm.display());
    }

    #[test]
    fn non_zero_exit_maps_to_silk_codec_with_stderr() {
        let dir = TestDir::new("exit-code");
        let input = dir.file("input.mp3", b"ID3");
        let output = dir.path().join("out.silk");

        let runner = Arc::new(FakeRunner::new(|_| Ok(failed(1, "boom: bad input"))));
        let codec = SilkCodec::with_runner(Arc::clone(&runner));

        let err = codec
            .encode_to_silk(&input, &output, 24_000)
            .expect_err("非零退出必须报错");
        match err {
            Error::SilkCodec(message) => {
                assert!(message.contains("退出码"), "{message}");
                assert!(message.contains("boom: bad input"), "{message}");
            }
            other => panic!("错误映射不对: {other:?}"),
        }
        assert_eq!(runner.calls().len(), 1, "ffmpeg 失败后不应调用 pts");
    }

    #[test]
    fn zero_sample_rate_is_rejected_before_any_process() {
        let dir = TestDir::new("rate");
        let input = dir.file("input.mp3", b"ID3");
        let output = dir.path().join("out.silk");

        let runner = Arc::new(FakeRunner::new(|_| Ok(ok())));
        let codec = SilkCodec::with_runner(Arc::clone(&runner));

        let err = codec
            .encode_to_silk(&input, &output, 0)
            .expect_err("采样率 0 必须报错");
        match err {
            Error::SilkCodec(message) => assert!(message.contains("采样率"), "{message}"),
            other => panic!("错误映射不对: {other:?}"),
        }
        assert!(runner.calls().is_empty(), "参数非法时不应启动进程");
    }

    #[test]
    fn custom_program_paths_are_used() {
        let dir = TestDir::new("custom-programs");
        let input = dir.file("input.mp3", b"ID3");
        let output = dir.path().join("out.silk");
        let output_for_fake = output.clone();

        let runner = Arc::new(FakeRunner::new(move |inv| match inv.program.as_str() {
            "/opt/ffmpeg" => {
                let pcm = PathBuf::from(inv.args.last().expect("ffmpeg 末参是 PCM 路径"));
                std::fs::write(&pcm, b"pcm-bytes").expect("写 PCM");
                Ok(ok())
            }
            "/opt/pts" => {
                std::fs::write(&output_for_fake, b"\x02#!SILK_V3").expect("写 silk");
                Ok(ok())
            }
            other => panic!("意外的程序: {other}"),
        }));
        let codec =
            SilkCodec::with_runner_and_programs(Arc::clone(&runner), "/opt/ffmpeg", "/opt/pts");

        assert_eq!(codec.ffmpeg_program(), "/opt/ffmpeg");
        assert_eq!(codec.pts_program(), "/opt/pts");
        codec
            .encode_to_silk(&input, &output, 16_000)
            .expect("编码成功");

        let calls = runner.calls();
        assert_eq!(calls[0].program, "/opt/ffmpeg");
        assert_eq!(calls[0].args[5], "16000", "ffmpeg -ar");
        assert_eq!(calls[1].program, "/opt/pts");
        assert_eq!(calls[1].args[5], "16000", "pts -s");
    }

    #[test]
    fn successful_exit_without_output_is_an_error() {
        let dir = TestDir::new("no-output");
        let input = dir.file("input.mp3", b"ID3");
        let output = dir.path().join("out.silk");

        let runner = Arc::new(FakeRunner::new(|_| Ok(ok())));
        let codec = SilkCodec::with_runner(runner);

        let err = codec
            .encode_to_silk(&input, &output, 24_000)
            .expect_err("产物缺失必须报错");
        match err {
            Error::SilkCodec(message) => assert!(message.contains("未产出"), "{message}"),
            other => panic!("错误映射不对: {other:?}"),
        }
    }

    #[test]
    fn real_command_runner_maps_missing_program() {
        let dir = TestDir::new("real-missing");
        let input = dir.file("input.mp3", b"ID3");
        let output = dir.path().join("out.silk");

        // 生产执行器 + 不存在的程序：只验证 NotFound → SilkUnavailable，不会真正启动进程。
        let codec = SilkCodec::with_programs("gensokyo-no-such-ffmpeg-7f3c", "pts");
        let err = codec
            .encode_to_silk(&input, &output, 24_000)
            .expect_err("不存在的程序必须报错");
        match err {
            Error::SilkUnavailable(message) => {
                assert!(
                    message.contains("gensokyo-no-such-ffmpeg-7f3c"),
                    "错误应点明程序: {message}"
                );
            }
            other => panic!("错误映射不对: {other:?}"),
        }
    }

    #[test]
    fn resolve_program_falls_back_on_missing_or_blank() {
        assert_eq!(
            resolve_program(PTS_ENV, "pts", |_| Some("/opt/pts".to_owned())),
            "/opt/pts"
        );
        assert_eq!(
            resolve_program(PTS_ENV, "pts", |_| Some("   ".to_owned())),
            "pts"
        );
        assert_eq!(resolve_program(PTS_ENV, "pts", |_| None), "pts");
    }
}
