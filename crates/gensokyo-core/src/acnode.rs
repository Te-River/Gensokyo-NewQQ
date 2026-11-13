//! 敏感词过滤：AC 自动机 + 三份词表（Go `acnode/` 对等物）。
//!
//! 对应关系：
//! - [`Acnode::load`] ≙ Go `acnode.init()`（`acnode.go:22-44` + `loadWordsIntoAC`
//!   `acnode.go:248-309`）：从工作目录读 `sensitive_words_in.txt` / `sensitive_words_out.txt`
//!   / `white.txt`，文件不存在就先释放空文件，每行按 `词####替换文本` 补全后回写。
//! - [`Acnode::filter_in`] / [`Acnode::filter_out`] ≙ `CheckWordIN` / `CheckWordOUT`
//!   （`acnode.go:325-376`）。
//! - [`install`] + 自由函数 [`filter_in`] / [`filter_out`] ≙ Go 的包级 `ac` / `acout` / `wac`：
//!   词表在进程内只加载一次，调用点按配置开关决定是否过滤。
//!
//! 唯一的行为差异：Go 在词表打不开时 `log.Fatalf` 直接退进程，这里降级为空表 + `warn`。

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::OnceLock;

use tracing::warn;

/// 入站词表文件名（相对工作目录，Go `acnode.go:28`）。
pub const IN_WORDS_FILE: &str = "sensitive_words_in.txt";
/// 出站词表文件名（Go `acnode.go:34`）。
pub const OUT_WORDS_FILE: &str = "sensitive_words_out.txt";
/// 白名单词表文件名（Go `acnode.go:40`）。
pub const WHITE_WORDS_FILE: &str = "white.txt";
/// 词与替换文本的分隔符（Go `strings.Split(line, "####")`）。
pub const REPLACE_SEPARATOR: &str = "####";
/// 单次过滤的字符数上限（Go `len([]rune(word)) > 5000`）。
pub const MAX_CHARS: usize = 5000;

const MISSING_WORD: &str = "错误：缺少 'word' 参数";
const TOO_LONG: &str = "错误：字符数超过最大限制（5000字符）";

/// 敏感词库：入站 / 出站 / 白名单三棵自动机（Go `ac` / `acout` / `wac`）。
#[derive(Debug)]
pub struct Acnode {
    inbound: Automaton,
    outbound: Automaton,
    white: Automaton,
}

impl Default for Acnode {
    fn default() -> Self {
        Self::empty()
    }
}

impl Acnode {
    /// 空词库（等价于三份词表都是空文件）。
    #[must_use]
    pub fn empty() -> Self {
        Self {
            inbound: Automaton::new(),
            outbound: Automaton::new(),
            white: Automaton::new(),
        }
    }

    /// 从 `dir` 读三份词表；文件缺失时释放空文件，任何 IO 失败只 `warn` 并按空表处理。
    #[must_use]
    pub fn load(dir: &Path, default_change_word: &str) -> Self {
        Self {
            inbound: load_words(&dir.join(IN_WORDS_FILE), default_change_word),
            outbound: load_words(&dir.join(OUT_WORDS_FILE), default_change_word),
            white: load_words(&dir.join(WHITE_WORDS_FILE), default_change_word),
        }
    }

    /// Go `CheckWordIN`：入站文本过滤（空串与超长返回错误串，与 Go 逐字一致）。
    #[must_use]
    pub fn filter_in(&self, text: &str) -> String {
        if text.is_empty() {
            tracing::debug!("敏感词入站过滤：缺少 word 参数");
            return MISSING_WORD.to_owned();
        }
        if text.chars().count() > MAX_CHARS {
            // Go 对超长的 base64 图片 CQ 码原样放行（`acnode.go:332-336`）。
            if text.contains("[CQ:image,file=base64://") {
                tracing::debug!("敏感词入站过滤：超长 base64 图片原样返回");
                return text.to_owned();
            }
            tracing::warn!("敏感词入站过滤：字符数超过 {MAX_CHARS}");
            return TOO_LONG.to_owned();
        }
        self.replace_with(&self.inbound, text)
    }

    /// Go `CheckWordOUT`：出站文本过滤（含 `base64://` 的文本整体放行）。
    #[must_use]
    pub fn filter_out(&self, text: &str) -> String {
        if text.is_empty() {
            tracing::debug!("敏感词出站过滤：缺少 word 参数");
            return MISSING_WORD.to_owned();
        }
        if text.contains("base64://") {
            return text.to_owned();
        }
        if text.chars().count() > MAX_CHARS {
            tracing::warn!("敏感词出站过滤：字符数超过 {MAX_CHARS}");
            return TOO_LONG.to_owned();
        }
        self.replace_with(&self.outbound, text)
    }

    fn replace_with(&self, automaton: &Automaton, text: &str) -> String {
        let whitelist = self.white.match_positions(text);
        let replacements = automaton.replacements(text, &whitelist);
        if replacements.is_empty() {
            return text.to_owned();
        }
        apply_replacements(text, replacements)
    }
}

static INSTALLED: OnceLock<Acnode> = OnceLock::new();

/// 装载进程级词库（Go `init()` 的等价物）；重复装载返回 `false` 并保留首次结果。
pub fn install(acnode: Acnode) -> bool {
    INSTALLED.set(acnode).is_ok()
}

/// 从 `dir` 装载进程级词库，等价于 [`install`] + [`Acnode::load`]。
pub fn load_into_process(dir: &Path, default_change_word: &str) -> bool {
    install(Acnode::load(dir, default_change_word))
}

/// 进程级词库是否已装载。
#[must_use]
pub fn is_installed() -> bool {
    INSTALLED.get().is_some()
}

/// 进程级入站过滤；未装载词库时原样返回（等价于空词表）。
#[must_use]
pub fn filter_in(text: &str) -> String {
    INSTALLED
        .get()
        .map_or_else(|| text.to_owned(), |acnode| acnode.filter_in(text))
}

/// 进程级出站过滤；未装载词库时原样返回（等价于空词表）。
#[must_use]
pub fn filter_out(text: &str) -> String {
    INSTALLED
        .get()
        .map_or_else(|| text.to_owned(), |acnode| acnode.filter_out(text))
}

/// 一次待执行的替换（Go `Replacement`，索引按字符计）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Replacement {
    start: usize,
    end: usize,
    text: String,
}

/// 一个已插入的词：长度 + 替换文本（Go `ACNode.isEnd` / `length` / `replaceText`）。
#[derive(Debug, Clone)]
struct Terminal {
    length: usize,
    replace: String,
}

#[derive(Debug)]
struct Node {
    children: HashMap<char, usize>,
    /// 失配指针；根指向自己（Go 根的 `fail` 是 `nil`）。
    fail: usize,
    terminal: Option<Terminal>,
}

impl Node {
    fn new() -> Self {
        Self {
            children: HashMap::new(),
            fail: 0,
            terminal: None,
        }
    }
}

/// 单棵 AC 自动机（Go `AhoCorasick`）。
#[derive(Debug)]
struct Automaton {
    nodes: Vec<Node>,
}

impl Automaton {
    fn new() -> Self {
        Self {
            nodes: vec![Node::new()],
        }
    }

    /// Go `Insert`：空词不落节点——匹配循环从根的子节点起走，根上的 `isEnd` 永远不会命中。
    fn insert(&mut self, word: &str, replace: &str) {
        let mut node = 0;
        let mut length = 0;
        for ch in word.chars() {
            node = match self.nodes[node].children.get(&ch) {
                Some(&next) => next,
                None => {
                    let next = self.nodes.len();
                    self.nodes.push(Node::new());
                    self.nodes[node].children.insert(ch, next);
                    next
                }
            };
            length += 1;
        }
        if node == 0 {
            return;
        }
        self.nodes[node].terminal = Some(Terminal {
            length,
            replace: replace.to_owned(),
        });
    }

    /// Go `BuildFailPointer`：BFS 逐层填失配指针，根的失配指针指向自己。
    fn build_fail(&mut self) {
        let mut queue = VecDeque::from([0]);
        while let Some(current) = queue.pop_front() {
            let children: Vec<(char, usize)> = self.nodes[current]
                .children
                .iter()
                .map(|(ch, next)| (*ch, *next))
                .collect();
            for (ch, child) in children {
                if current != 0 {
                    let mut fail = self.nodes[current].fail;
                    loop {
                        if let Some(&next) = self.nodes[fail].children.get(&ch) {
                            self.nodes[child].fail = next;
                            break;
                        }
                        if fail == 0 {
                            self.nodes[child].fail = 0;
                            break;
                        }
                        fail = self.nodes[fail].fail;
                    }
                }
                queue.push_back(child);
            }
        }
    }

    /// 沿失配链从最长的后缀匹配开始找：每个字符位置只取第一个不在白名单内的命中
    /// （Go `FilterWithWhitelist` 的 `tmp` 循环 + `break`）。
    fn replacements(&self, text: &str, whitelist: &[(usize, usize)]) -> Vec<Replacement> {
        let mut found = Vec::new();
        let mut node = 0;
        for (index, ch) in text.chars().enumerate() {
            while node != 0 && !self.nodes[node].children.contains_key(&ch) {
                node = self.nodes[node].fail;
            }
            if let Some(&next) = self.nodes[node].children.get(&ch) {
                node = next;
            }
            let mut walk = node;
            while walk != 0 {
                if let Some(terminal) = &self.nodes[walk].terminal
                    && !is_whitelisted(index, terminal.length, whitelist)
                {
                    found.push(Replacement {
                        start: index + 1 - terminal.length,
                        end: index,
                        text: terminal.replace.clone(),
                    });
                    break;
                }
                walk = self.nodes[walk].fail;
            }
        }
        found
    }

    /// Go `MatchPositions`：白名单命中的全部区间（不 `break`，重叠的后缀也收）。
    fn match_positions(&self, text: &str) -> Vec<(usize, usize)> {
        let mut positions = Vec::new();
        let mut node = 0;
        for (index, ch) in text.chars().enumerate() {
            while node != 0 && !self.nodes[node].children.contains_key(&ch) {
                node = self.nodes[node].fail;
            }
            if let Some(&next) = self.nodes[node].children.get(&ch) {
                node = next;
            }
            let mut walk = node;
            while walk != 0 {
                if let Some(terminal) = &self.nodes[walk].terminal {
                    positions.push((index + 1 - terminal.length, index));
                }
                walk = self.nodes[walk].fail;
            }
        }
        positions
    }
}

/// Go `i-pos.Start+1 >= tmp.length && i <= pos.End`：命中区间完全落在某个白名单区间内。
fn is_whitelisted(index: usize, length: usize, whitelist: &[(usize, usize)]) -> bool {
    whitelist
        .iter()
        .any(|(start, end)| index + 1 >= start + length && index <= *end)
}

/// Go `mergeOverlappingReplacements`：按起点排序后合并重叠区间（后者覆盖前者）。
fn merge_overlapping(mut replacements: Vec<Replacement>) -> Vec<Replacement> {
    replacements.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut merged: Vec<Replacement> = Vec::with_capacity(replacements.len());
    for current in replacements {
        match merged.last_mut() {
            Some(last) if current.start <= last.end => {
                if current.end > last.end {
                    last.end = current.end;
                    last.text = current.text;
                }
            }
            _ => merged.push(current),
        }
    }
    merged
}

/// Go `applyReplacements`：按合并后的区间拼回文本（索引按字符计）。
fn apply_replacements(text: &str, replacements: Vec<Replacement>) -> String {
    let runes: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut last = 0;
    for replacement in merge_overlapping(replacements) {
        result.extend(runes[last..replacement.start].iter());
        result.push_str(&replacement.text);
        last = replacement.end + 1;
    }
    result.extend(runes[last..].iter());
    result
}

/// Go `loadWordsIntoAC`：读词表 → 补全每行的替换文本 → 回写 → 建自动机。
fn load_words(path: &Path, default_change_word: &str) -> Automaton {
    let mut automaton = Automaton::new();
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if let Err(err) = std::fs::write(path, "") {
                warn!(path = %path.display(), error = %err, "敏感词表不存在且创建失败，按空表处理");
                return automaton;
            }
            String::new()
        }
        Err(err) => {
            warn!(path = %path.display(), error = %err, "敏感词表读取失败，按空表处理");
            return automaton;
        }
    };

    let mut rewritten = String::with_capacity(text.len() + 16);
    for line in text.lines() {
        // Go `bufio.ScanLines`：按 `\n` 切并剥掉行尾 `\r`。
        let parts: Vec<&str> = line.split(REPLACE_SEPARATOR).collect();
        let word = parts[0];
        let specified = parts.get(1).copied().unwrap_or_default();
        let replace = if specified.is_empty() {
            // 缺 `####替换文本` 时补默认值后回写原文件（`acnode.go:277-283`）。
            let _ = writeln!(rewritten, "{word}{REPLACE_SEPARATOR}{default_change_word}");
            default_change_word
        } else {
            let _ = writeln!(rewritten, "{line}");
            specified
        };
        if !word.is_empty() {
            automaton.insert(word, replace);
            // 同时插入 Unicode 转义形态，让字面量 `\uXXXX` 文本也能命中（`acnode.go:289-290`）。
            automaton.insert(&unicode_escape(word), replace);
        }
    }

    if let Err(err) = std::fs::write(path, rewritten.as_bytes()) {
        warn!(path = %path.display(), error = %err, "敏感词表回写失败");
    }
    automaton.build_fail();
    automaton
}

/// Go `convertToUnicodeEscape`：按 UTF-16 码元写成 `\uxxxx`（小写十六进制）。
fn unicode_escape(word: &str) -> String {
    let mut escaped = String::with_capacity(word.len() * 6);
    let mut units = [0u16; 2];
    for ch in word.chars() {
        for unit in ch.encode_utf16(&mut units) {
            let _ = write!(escaped, "\\u{unit:04x}");
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        Acnode, IN_WORDS_FILE, MAX_CHARS, OUT_WORDS_FILE, WHITE_WORDS_FILE, filter_in, filter_out,
        install, is_installed,
    };

    /// 临时工作目录（跑完即删，不落仓库）。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            static SEQ: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "gensokyo-acnode-{}-{}-{name}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).expect("建临时目录失败");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, file: &str, body: &str) {
            std::fs::write(self.0.join(file), body).expect("写词表失败");
        }

        fn load(&self, default_change_word: &str) -> Acnode {
            Acnode::load(&self.0, default_change_word)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_files_are_created_empty_and_filter_passes_through() {
        let scratch = Scratch::new("missing");
        let acnode = scratch.load("*");

        for file in [IN_WORDS_FILE, OUT_WORDS_FILE, WHITE_WORDS_FILE] {
            let path = scratch.path().join(file);
            assert!(path.exists(), "{file} 应被释放");
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                "",
                "{file} 应为空表"
            );
        }
        assert_eq!(acnode.filter_in("正常文本"), "正常文本");
        assert_eq!(acnode.filter_out("正常文本"), "正常文本");
    }

    #[test]
    fn hit_and_miss_use_the_default_change_word() {
        let scratch = Scratch::new("hit");
        scratch.write(IN_WORDS_FILE, "敏感词\n");
        scratch.write(OUT_WORDS_FILE, "禁发\n");
        let acnode = scratch.load("*");

        assert_eq!(acnode.filter_in("这是敏感词啊"), "这是*啊");
        assert_eq!(acnode.filter_in("这是正常文本"), "这是正常文本");
        assert_eq!(acnode.filter_out("禁发内容"), "*内容");
        // 入站/出站词表互不串用
        assert_eq!(acnode.filter_in("禁发内容"), "禁发内容");
        assert_eq!(acnode.filter_out("这是敏感词啊"), "这是敏感词啊");
    }

    #[test]
    fn explicit_replacement_text_wins_and_file_is_rewritten() {
        let scratch = Scratch::new("replace");
        scratch.write(IN_WORDS_FILE, "坏词####好词\n裸词\n");
        let acnode = scratch.load("#");

        assert_eq!(acnode.filter_in("坏词"), "好词");
        assert_eq!(acnode.filter_in("裸词"), "#");
        assert_eq!(
            std::fs::read_to_string(scratch.path().join(IN_WORDS_FILE)).unwrap(),
            "坏词####好词\n裸词#####\n",
            "缺替换文本的行要补全后回写"
        );
    }

    #[test]
    fn whitelist_protects_contained_matches_only() {
        let scratch = Scratch::new("white");
        scratch.write(IN_WORDS_FILE, "测试\n");
        scratch.write(WHITE_WORDS_FILE, "测试用例\n");
        let acnode = scratch.load("*");

        assert_eq!(acnode.filter_in("这是测试用例"), "这是测试用例");
        assert_eq!(acnode.filter_in("这是测试"), "这是*");
        // 白名单区间之外的同一词仍然命中
        assert_eq!(acnode.filter_in("测试用例和测试"), "测试用例和*");
    }

    #[test]
    fn multiple_words_and_overlap_merge_like_go() {
        let scratch = Scratch::new("overlap");
        scratch.write(OUT_WORDS_FILE, "ab####X\nbc####Y\n");
        let acnode = scratch.load("*");

        assert_eq!(acnode.filter_out("abc"), "Y");
        assert_eq!(acnode.filter_out("ab bc"), "X Y");
    }

    #[test]
    fn matching_is_case_and_width_sensitive_like_go() {
        let scratch = Scratch::new("case");
        scratch.write(IN_WORDS_FILE, "abc\n");
        let acnode = scratch.load("*");

        assert_eq!(acnode.filter_in("ABC abc"), "ABC *");
        // 全角字母不是半角字母（Go 不做归一化）
        assert_eq!(acnode.filter_in("ａｂｃ"), "ａｂｃ");
    }

    #[test]
    fn unicode_escaped_literal_form_is_also_matched() {
        let scratch = Scratch::new("escape");
        scratch.write(IN_WORDS_FILE, "我\n");
        let acnode = scratch.load("*");

        assert_eq!(acnode.filter_in("我"), "*");
        assert_eq!(acnode.filter_in("\\u6211"), "*");
    }

    #[test]
    fn empty_vocabulary_keeps_text_verbatim() {
        let scratch = Scratch::new("empty");
        scratch.write(IN_WORDS_FILE, "\n\n");
        let acnode = scratch.load("*");

        assert_eq!(acnode.filter_in("任意文本"), "任意文本");
        assert_eq!(acnode.filter_out("任意文本"), "任意文本");
    }

    #[test]
    fn guards_follow_go() {
        let scratch = Scratch::new("guards");
        scratch.write(OUT_WORDS_FILE, "坏\n");
        let acnode = scratch.load("*");

        assert_eq!(acnode.filter_in(""), "错误：缺少 'word' 参数");
        assert_eq!(acnode.filter_out(""), "错误：缺少 'word' 参数");

        let base64 = format!("base64://{}", "A".repeat(MAX_CHARS));
        assert_eq!(
            acnode.filter_out(&base64),
            base64,
            "出站遇 base64:// 整体放行"
        );

        let long = "坏".repeat(MAX_CHARS + 1);
        assert_eq!(
            acnode.filter_out(&long),
            "错误：字符数超过最大限制（5000字符）"
        );
        // 入站只在超长时才放行 base64 图片 CQ 码
        let long_image = format!("[CQ:image,file=base64://{}", "A".repeat(MAX_CHARS));
        assert_eq!(acnode.filter_in(&long_image), long_image);
        assert_eq!(
            acnode.filter_in(&long),
            "错误：字符数超过最大限制（5000字符）"
        );
    }

    #[test]
    fn process_level_helpers_degrade_to_passthrough_then_filter() {
        if !is_installed() {
            assert_eq!(filter_in("违禁字样"), "违禁字样");
            assert_eq!(filter_out("违禁字样"), "违禁字样");
        }

        let scratch = Scratch::new("global");
        scratch.write(IN_WORDS_FILE, "违禁字样####□\n");
        scratch.write(OUT_WORDS_FILE, "违禁字样####○\n");
        assert!(install(scratch.load("*")), "首次装载应成功");
        assert!(is_installed());
        assert_eq!(filter_in("违禁字样"), "□");
        assert_eq!(filter_out("违禁字样"), "○");
        // 重复装载保留首次结果
        assert!(!install(Acnode::empty()));
        assert_eq!(filter_in("违禁字样"), "□");
    }
}
