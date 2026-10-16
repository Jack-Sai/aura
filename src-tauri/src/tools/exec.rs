//! 命令执行工具（v0.3.1）
//!
//! 在工作区根目录下执行命令，捕获 stdout/stderr 回填给模型。
//!
//! **安全边界说明**：真正的护栏只有「工作目录锁定在工作区内」这一条。
//! 危险命令黑名单属于尽力而为的兜底，挡不住 `python -c "import shutil; ..."`
//! 这类通过解释器间接删除文件的操作。若后续要支持无人值守的长程任务，
//! 应升级为命令白名单或容器隔离。
//!
//! 本工具由 `tools::execute` 在 `spawn_blocking` 中调用，阻塞子进程不会
//! 占用 tokio worker，SSE 流式输出与停止按钮保持响应。
//!
//! **子进程的管道必须在等待退出之前抽干**（见 [`PipeDrain`]）：匿名管道
//! 缓冲区只有几 KB，子进程写满后阻塞在 `write()` 上永不退出，而父进程在
//! `try_wait()` 轮询里空转到超时。这是 v0.5.1 修复的性能缺陷——实测输出
//! 330KB 的命令，改动前 >12s 未退出（走超时分支），改动后 160ms 完成。
//! `pnpm install`、`cargo build` 这类输出较多的命令几乎必然触发。

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use super::ToolOutput;

/// 合并输出的字符上限，超出后头尾截断，防止撑爆模型上下文。
const MAX_OUTPUT: usize = 5000;
/// 每段（头/尾）保留的字符数。
const KEEP_EACH_SIDE: usize = MAX_OUTPUT * 2 / 3;
/// 默认执行超时。
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// 超时上限，避免模型传入极大值导致任务挂死。
const MAX_TIMEOUT_MS: u64 = 600_000;

/// 删除类动词。**必须与递归/强制标志同时出现**才判定危险。
///
/// 单独出现并不危险：`rm file.txt`、`del app.log` 都是正常操作，
/// 而 `rd`/`ri` 这类短别名还可能出现在文件名或 `mkdir` 里，
/// 因此统一按词边界匹配，且要求后接危险标志。
const DELETE_VERBS: &[&str] = &[
    "remove-item",
    "remove-itemproperty",
    "rmdir",
    "rd",
    "ri",
    "rm",
    "del",
    "erase",
];

/// 递归或强制标志。与 [`DELETE_VERBS`] 组合才构成危险操作。
const DESTRUCTIVE_FLAGS: &[&str] = &["-recurse", "-rec", "-force", "/s", "/q"];

/// 独立危险项：命中即拦，无需组合。
const DANGEROUS: &[(&str, &str)] = &[
    ("rm -rf /", "递归删除根目录"),
    ("rm -rf ~", "递归删除用户主目录"),
    ("rm -rf *", "递归删除当前目录全部内容"),
    ("rmdir /s", "Windows 递归删除目录"),
    ("del /f", "Windows 强制删除文件"),
    ("del /q", "Windows 静默删除文件"),
    ("format ", "磁盘格式化"),
    ("format-volume", "格式化卷"),
    ("mkfs", "文件系统格式化"),
    ("diskpart", "磁盘分区操作"),
    ("clear-disk", "清除磁盘"),
    ("initialize-disk", "初始化磁盘"),
    ("shutdown", "系统关机"),
    ("stop-computer", "关机"),
    ("restart-computer", "重启"),
    ("halt", "系统停机"),
    ("poweroff", "系统断电"),
    ("reg delete", "删除注册表"),
    ("takeown", "夺取文件所有权"),
    ("icacls", "修改文件权限"),
    (":(){", "fork 炸弹"),
    ("dd if=", "裸设备写入"),
    ("chmod -r 777 /", "递归放开根目录权限"),
    ("chown -r", "递归变更属主"),
    ("clear-content", "清空文件内容"),
    ("invoke-webrequest", "疑似下载并执行远程脚本"),
    ("iex", "疑似下载并执行远程脚本"),
    ("invoke-expression", "执行动态构造的命令"),
];

/// 在 `hay` 中按词边界查找 `word`，返回其起始字节偏移。
///
/// 词边界定义为：前后都不是字母、数字或下划线。
/// 这样 `rd` 不会命中 `mkdir` / `third`，`rm` 不会命中 `firm`。
fn find_word(hay: &str, word: &str) -> Option<usize> {
    let bytes = hay.as_bytes();
    let w = word.as_bytes();
    if w.is_empty() || hay.len() < w.len() {
        return None;
    }
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    for i in 0..=(bytes.len() - w.len()) {
        if &bytes[i..i + w.len()] != w {
            continue;
        }
        let before_ok = i == 0 || !is_word(bytes[i - 1]);
        let after_idx = i + w.len();
        let after_ok = after_idx >= bytes.len() || !is_word(bytes[after_idx]);
        if before_ok && after_ok {
            return Some(i);
        }
    }
    None
}

/// 检测「删除动词 + 递归/强制标志」的组合。
///
/// 解决黑名单的根本缺口：原实现只做小写字面子串匹配，
/// 而 `powershell -Command "Remove-Item -Recurse -Force x"` 把命令
/// 当**数据**传给解释器，整条执行语义对匹配器不可见，
/// 于是被拦的 `rmdir /s` 换个 `Remove-Item` 就绕过去了。
///
/// 这里改为识别语义组合而非命令字面量，因此解释器包装同样会被命中。
fn check_destructive_delete(normalized: &str) -> Option<&'static str> {
    for verb in DELETE_VERBS {
        let Some(pos) = find_word(normalized, verb) else {
            continue;
        };
        let tail = &normalized[pos + verb.len()..];
        if DESTRUCTIVE_FLAGS
            .iter()
            .any(|flag| find_word(tail, flag).is_some())
        {
            return Some("递归或强制删除");
        }
        // `-rf` / `-fr` / `-frx` 这类紧凑短横线组合：拆成单个标志就漏了。
        // 只认「短横线后紧跟 r/f/s/q 字母」的形式，避免把 `--force` 之类的
        // 正常参数误当成递归标志
        if let Some(i) = tail.find('-') {
            let flags: Vec<char> = tail[i + 1..]
                .chars()
                .take_while(|c| matches!(c, 'r' | 'f' | 's' | 'q'))
                .collect();
            if flags.contains(&'r') || flags.contains(&'f') {
                return Some("递归或强制删除");
            }
        }
    }
    None
}

/// 命中黑名单时返回原因说明，否则返回 None。
pub fn check_dangerous(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    // shell 布尔运算符会切断命令语义，绕过朴素的子串匹配
    let normalized = lower
        .replace("&&", " ")
        .replace("||", " ")
        .replace(';', " ")
        .replace('|', " ")
        .replace('\n', " ");
    DANGEROUS
        .iter()
        .find(|(pat, _)| normalized.contains(pat))
        .map(|(_, reason)| *reason)
        .or_else(|| check_destructive_delete(&normalized))
}

/// 解码子进程输出字节。
///
/// 中文 Windows 的 cmd 默认按 GBK(936) 输出，直接按 UTF-8 解码会得到一串
/// `�`，模型读到的是乱码。这里按「UTF-8 优先，失败回退 GBK」处理：
/// 多数现代程序（Go/Rust/Node）会输出 UTF-8，而 cmd 内建命令与不少脚本
/// 是 GBK。
///
/// 曾尝试前置 `chcp 65001` 统一代码页，但**不可靠**：chcp 修改的是控制台的
/// 共享状态，多个 cmd 进程并发或先后执行时会互相干扰，实测出现同一段代码
/// 单独运行正常、跟在另一个 cmd 之后反而输出 GBK 的情况。
fn decode_bytes(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => return s.to_string(),
        Err(_) => {}
    }
    // UTF-8 解码失败 → 视作 GBK
    let (text, _, _) = encoding_rs::GBK.decode(raw);
    text.into_owned()
}

/// 头尾截断，超长时中间标注省略字符数。
pub fn truncate_output(raw: &str, max: usize) -> (String, bool) {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() <= max {
        return (raw.to_string(), false);
    }
    let head: String = chars[..KEEP_EACH_SIDE.min(chars.len())].iter().collect();
    let tail_start = chars.len().saturating_sub(max - KEEP_EACH_SIDE);
    let tail: String = chars[tail_start..].iter().collect();
    let omitted = tail_start - KEEP_EACH_SIDE.min(tail_start);
    (format!("{}\n…（中间省略 {} 字符）…\n{}", head, omitted, tail), true)
}

fn read_args<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("缺少参数 `{}` 或类型错误", key))
}

/// `run_script`：在工作区根目录执行命令。
pub fn run_script(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let command = read_args(args, "command")?.trim();
    if command.is_empty() {
        return Err("`command` 不能为空".into());
    }
    if let Some(reason) = check_dangerous(command) {
        return Err(format!(
            "命令被安全策略拦截（疑似{}）：{}\n\
             如确需执行，请调整命令写法以避开该模式。",
            reason, command
        ));
    }

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1_000, MAX_TIMEOUT_MS))
        .unwrap_or(DEFAULT_TIMEOUT_MS);

    let mut cmd = build_command(root, command);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("无法启动进程: {}", e))?;

    // ★ 必须**在**开始轮询之前就把管道交给后台线程读空。
    //
    // 匿名管道缓冲区只有几 KB，子进程写满后会阻塞在 `write()` 上永不退出；
    // 父进程此时只在 `try_wait()` 里轮询，双方互等到超时才收场。
    // 实测同一条输出 330KB 的命令：本实现 >12s 未退出，改为边读边等后 160ms 完成。
    //
    // 不能改用 `wait_with_output()`：它确实会先读管道（因而不会死锁），
    // 但没有超时与取消能力，模型传 `timeout_ms: 600000` 时界面会真的卡十分钟。
    let drain = PipeDrain::attach(&mut child);

    // 轮询等待：既能超时中断，也能在等待期间响应取消（由上层 tokio select 处理）
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let partial = drain.join();
                    let (out, truncated) = truncate_output(&partial, MAX_OUTPUT);
                    return Ok(ToolOutput {
                        label: format!("Exec '{}'（超时）", first_line(command)),
                        result: format!(
                            "命令执行超时（{} 秒）已终止。\n--- 部分输出 ---\n{}",
                            timeout_ms / 1000,
                            if truncated {
                                format!("{}\n（输出已截断）", out)
                            } else {
                                out
                            }
                        ),
                    });
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("等待进程失败: {}", e)),
        }
    };

    let raw_output = drain.join();
    let (out, truncated) = truncate_output(&raw_output, MAX_OUTPUT);
    let body = if out.trim().is_empty() {
        "（命令无输出）".to_string()
    } else if truncated {
        format!("{}\n（输出已截断，仅保留头尾）", out)
    } else {
        out
    };

    // 退出码必须回填：模型据此判断构建/测试是否通过
    let code_text = match exit_code {
        Some(0) => "退出码 0（成功）".to_string(),
        Some(c) => format!("退出码 {}（失败）", c),
        None => "进程被信号终止".to_string(),
    };

    Ok(ToolOutput {
        label: format!("Exec '{}'", first_line(command)),
        result: format!("{}\n--- 输出 ---\n{}", code_text, body),
    })
}

/// 通过环境变量传递原始命令串。
///
/// **不能直接用 `Command::arg(command)`**：Rust 会按 CRT 规则把命令里的
/// `"` 转义成 `\"`，而 cmd 有自己的一套引号解析规则，两者不兼容——
/// 结果是 `git commit -m "Add MST code"` 被当成一个名为 `\"echo ...\"`
/// 的可执行文件去查找，直接失败。这正是模型反复重试 `git commit -m` 的原因。
///
/// 实测三种方案：
/// - `arg("/C")` + 拼接前缀 + 整条引号包裹 → cmd 找不到命令（引号被当字面量）
/// - `arg("/S")` + `arg("/C")` + 包裹 → 同样失败
/// - **环境变量** → 引号、中文、管道均正确
#[cfg(windows)]
fn build_command(root: &Path, command: &str) -> std::process::Command {
    // **命令必须经环境变量传入**，不能拼接进命令行字面量：
    //
    // Rust 会在创建进程时按 CRT 规则把参数里的 `"` 转义成 `\"`，而 cmd 有
    // 一套自己的引号解析规则，两者不兼容 → `git commit -m "Add MST code"`
    // 会被当成名为 `\"echo ...\"` 的可执行文件去查找，直接失败。
    // 这正是模型反复重试 `git commit -m` 却始终不成功的原因。
    //
    // 中文改由 `decode_bytes` 按 GBK 解码处理，不依赖 `chcp`——
    // chcp 修改的是控制台共享状态，并发或先后执行多个 cmd 时会互相干扰。
    let mut cmd = std::process::Command::new("cmd");
    cmd.arg("/C")
        .arg("%AURA_CMD%")
        .env("AURA_CMD", command)
        .current_dir(root);
    cmd
}

#[cfg(not(windows))]
fn build_command(root: &Path, command: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(command).current_dir(root);
    cmd
}

/// 取命令首行用于 UI 文案，避免超长命令撑爆动作卡片。
fn first_line(command: &str) -> String {
    let line = command.lines().next().unwrap_or(command).trim();
    if line.chars().count() > 60 {
        let mut s: String = line.chars().take(60).collect();
        s.push('…');
        s
    } else {
        line.to_string()
    }
}

/// 单条管道最多保留的字节数。
///
/// 超过部分**丢弃但继续读到 EOF** —— 必须排空管道，否则子进程会再次阻塞在
/// `write()` 上，最终走超时分支。已有 [`MAX_OUTPUT`] 的头尾截断兜底，
/// 这里只是防止异常输出把内存吃光。
const MAX_PIPE_BYTES: usize = 8 * 1024 * 1024;

/// 后台读取子进程的 stdout / stderr，直到 EOF。
///
/// **必须在等待子进程退出之前启动**，否则管道写满会造成互等死锁，
/// 详见 [`run_script`] 里的说明。
struct PipeDrain {
    out: Option<std::thread::JoinHandle<Vec<u8>>>,
    err: Option<std::thread::JoinHandle<Vec<u8>>>,
}

impl PipeDrain {
    /// 接管子进程的两条管道并起线程持续读取。
    fn attach(child: &mut std::process::Child) -> Self {
        Self {
            out: child.stdout.take().map(|p| std::thread::spawn(move || read_capped(p))),
            err: child.stderr.take().map(|p| std::thread::spawn(move || read_capped(p))),
        }
    }

    /// 等两个读取线程结束，按 stdout → stderr 顺序合并解码。
    ///
    /// 子进程退出后写端全部关闭，读取线程会自然收到 EOF，因此即便走了
    /// 超时分支（先 `kill` 再 `wait`）也不会卡在 `join` 上。
    fn join(self) -> String {
        let mut buf = String::new();
        for handle in [self.out, self.err].into_iter().flatten() {
            // 读取线程 panic 不应让整条工具调用失败，跳过该路输出即可
            let Ok(raw) = handle.join() else { continue };
            let text = decode_bytes(&raw);
            if text.trim().is_empty() {
                continue;
            }
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(&text);
        }
        buf
    }
}

/// 读到 EOF 为止，但只保留前 [`MAX_PIPE_BYTES`] 字节。
fn read_capped<R: std::io::Read>(mut r: R) -> Vec<u8> {
    let mut kept: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if kept.len() < MAX_PIPE_BYTES {
                    let room = MAX_PIPE_BYTES - kept.len();
                    kept.extend_from_slice(&chunk[..n.min(room)]);
                }
            }
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_dangerous_commands() {
        for bad in [
            "rm -rf /",
            "rm -rf ~",
            "sudo rm -rf / --no-preserve-root",
            "mkfs.ext4 /dev/sda",
            "format c:",
            "shutdown /s",
            "del /f file.txt",
            "rmdir /s /q folder",
            ":(){ :|:& };:",
        ] {
            assert!(
                check_dangerous(bad).is_some(),
                "应拦截危险命令: {}",
                bad
            );
        }
    }

    #[test]
    fn blocks_dangerous_behind_shell_operators() {
        // shell 运算符不能成为绕过手段
        assert!(check_dangerous("echo hi && rm -rf /").is_some());
        assert!(check_dangerous("echo hi; rm -rf /").is_some());
        assert!(check_dangerous("echo hi || mkfs /dev/sda").is_some());
        assert!(check_dangerous("echo a\nrm -rf /").is_some());
    }

    #[test]
    fn allows_ordinary_commands() {
        for ok in [
            "cargo build",
            "pnpm install",
            "npm run test",
            "git status",
            "git diff --stat",
            "python -c \"print(1)\"",
            "rm file.txt",
            "ls -la",
            "grep -rn 'TODO' src/",
            "echo hello world",
        ] {
            assert!(
                check_dangerous(ok).is_none(),
                "不应拦截正常命令: {}",
                ok
            );
        }
    }

    // ---- 管道死锁回归（v0.5.1）----

    /// 生成约 52KB 输出的**单行**命令，不依赖外部工具、不写任何文件。
    ///
    /// 行宽取 64 字符 × 800 行。管道缓冲区只有几 KB，52KB 已足够跨过
    /// 死锁阈值，不必堆到 128KB——那只会让本套件白等好几秒。
    /// 末行用 `if` 判断输出标记，使整条命令只有一个逻辑行：
    /// `cmd /C %AURA_CMD%` 只执行第一行（见下方「多行命令」测试），
    /// 标记若另起一行根本不会被执行，测试就会得到假阴性。
    #[cfg(windows)]
    fn big_output_command() -> String {
        let pad = "A".repeat(64);
        format!(
            "for /L %i in (1,1,800) do @if %i==800 (echo AURA_TAIL_MARKER) else (echo {pad})"
        )
    }

    /// 大输出命令必须在超时前正常结束，且截断后**保留尾部**。
    ///
    /// 三件事一次验完，因为它们共用同一条命令：
    /// 1. **不死锁** —— 修复前子进程写满管道后阻塞在 `write()`，父进程空转到
    ///    超时，走「已终止」分支；现在必须拿到退出码 0。
    /// 2. **触发截断** —— 52KB 远超 MAX_OUTPUT(5000 字符)。
    /// 3. **尾部保留** —— 编译错误常出现在输出中后段，头尾截断的语义一旦
    ///    被改坏，模型会看不到真正的报错位置。
    #[cfg(windows)]
    #[test]
    fn run_script_handles_large_output_without_deadlock() {
        let args = serde_json::json!({
            "command": big_output_command(),
            "timeout_ms": 20_000,
        });
        let out = run_script(std::path::Path::new("."), &args).expect("命令应正常完成");

        assert!(
            out.result.contains("退出码 0"),
            "大输出命令应在超时前正常结束（修复前会卡到超时），实际返回：{}",
            &out.result[..out.result.len().min(300)]
        );
        assert!(
            out.result.contains("输出已截断"),
            "52KB 输出应触发截断，实际返回：{}",
            &out.result[..out.result.len().min(200)]
        );
        assert!(
            out.result.contains("AURA_TAIL_MARKER"),
            "截断后必须保留尾部标记，否则模型看不到最后的输出"
        );
    }

    /// 超时分支下管道同样要能被抽干：`join` 不能卡死。
    #[cfg(windows)]
    #[test]
    fn run_script_timeout_still_returns_partial_output() {
        // ping 逐秒输出一行，共约 29 秒，必然触发超时；
        // 输出写向 stdout 而非 nul，确保「部分输出」确有内容可回填
        let args = serde_json::json!({
            "command": "ping -n 30 127.0.0.1",
            "timeout_ms": 2_500,
        });
        let out = run_script(std::path::Path::new("."), &args).expect("超时应作为正常结果返回");

        assert!(
            out.result.contains("超时"),
            "应返回超时提示，实际：{}",
            &out.result[..out.result.len().min(200)]
        );
        assert!(
            out.result.contains("部分输出"),
            "超时应回填已产生的部分输出，而不是空白"
        );
        assert!(
            out.result.contains("127.0.0.1"),
            "kill 前已产生的 ping 输出应被抽干并回填"
        );
    }

    /// 记录一个既有行为：`cmd /C %AURA_CMD%` **只执行第一行**。
    ///
    /// 这不是缺陷而是防线——模型若输出「无害命令换行破坏性命令」，
    /// 第二行不会被执行（黑名单那边还额外把 `\n` 归一为空格，拦得更严）。
    /// 但模型不知道这件事，因此提示词需引导命令写成单行。
    #[cfg(windows)]
    #[test]
    fn only_first_line_of_multiline_command_runs() {
        let args = serde_json::json!({ "command": "echo FIRST_LINE\necho SECOND_LINE" });
        let out = run_script(std::path::Path::new("."), &args).unwrap();

        assert!(
            out.result.contains("FIRST_LINE"),
            "第一行应被执行"
        );
        assert!(
            !out.result.contains("SECOND_LINE"),
            "多行命令的后续行不会执行——模型输出多行命令时会静默丢失"
        );
    }

    #[test]
    fn conservative_about_dangerous_substrings() {
        // 黑名单是「宁可误拦」策略：即使危险词出现在引号里（shell 仍可能展开），
        // 也一律拦截。误拦的代价远小于漏拦。
        assert!(check_dangerous("echo 'rm -rf /'").is_some());
        assert!(check_dangerous("echo 'mkfs'").is_some());
    }

    // ---- 解释器绕过：v0.4.4 修复的核心缺口 ----
    //
    // 原实现只做小写字面子串匹配，而 `powershell -Command "..."` 把命令
    // 当**数据**传给解释器，整条执行语义对匹配器不可见。于是模型被拦下
    // `rmdir /s /q x` 后，改用 `Remove-Item -Recurse -Force x` 即可绕���，
    // 用户实测确认了这个路径。

    #[test]
    fn blocks_destructive_delete_behind_powershell() {
        for cmd in [
            "powershell -Command \"Remove-Item -Recurse -Force graph-algorithms\"",
            "powershell -Command 'Remove-Item -Recurse x'",
            "pwsh -c \"rd /s /q x\"",
            "pwsh -NoProfile -Command \"Remove-Item -Recurse -Force $env:TEMP\"",
            "powershell.exe -Command \"ri -Recurse -Force x\"",
            "cmd /c \"rmdir /s /q x\"",
        ] {
            assert!(
                check_dangerous(cmd).is_some(),
                "解释器包装的递归删除必须被拦下: {cmd}"
            );
        }
    }

    #[test]
    fn blocks_destructive_delete_across_shell_operators() {
        // 运算符拼接不得成为绕过路径
        for cmd in [
            "cd x && Remove-Item -Recurse -Force y",
            "echo hi; remove-item -force z",
            "echo a | rm -rf node_modules",
        ] {
            assert!(
                check_dangerous(cmd).is_some(),
                "运算符拼接的递归删除必须被拦下: {cmd}"
            );
        }
    }

    /// v0.4.4 的行为变更：紧凑短横线递归删除一律拦截。
    ///
    /// 此前 `rm -rf <具体目录>` 是放行的（原黑名单只有 `rm -rf /`、`rm -rf ~`
    /// 和 `rm -rf *` 三条）。但 `rm -rf node_modules` 同样会递归删除整个目录树，
    /// 用户实测中模型正是用这类命令把刚建好的项目目录删掉重来。
    ///
    /// 代价：清理 node_modules 这类操作现在需要用户手动执行。
    /// 这是刻意的取舍——误拦的代价远小于误删。
    #[test]
    fn recursive_delete_with_named_target_is_now_blocked() {
        for cmd in [
            "rm -rf node_modules",
            "rm -rf build",
            "rm -rf ./dist",
            "rm -rf /tmp/foo",
        ] {
            assert!(
                check_dangerous(cmd).is_some(),
                "带具体路径的递归删除现在也应拦下: {cmd}"
            );
        }
    }

    /// 短别名必须按词边界匹配，否则会大面积误伤正常命令。
    #[test]
    fn short_aliases_do_not_match_inside_words() {
        for ok in [
            "mkdir build",
            "rmdir2 utility",
            "cargo build --release",
            "npm run third",
            "git rm --cached node_modules",
            "echo removing files",
            "firmware update",
        ] {
            assert!(
                check_dangerous(ok).is_none(),
                "词边界内的别名不应误判: {ok}"
            );
        }
    }

    /// 删除动词单独出现是合法的，只在与递归/强制标志组合时才危险。
    #[test]
    fn plain_delete_without_flags_stays_allowed() {
        for ok in [
            "rm file.txt",
            "rm old.log",
            "del app.log",
            "rd folder",
        ] {
            assert!(
                check_dangerous(ok).is_none(),
                "无递归/强制标志的删除应放行: {ok}"
            );
        }
    }

    #[test]
    fn blocks_other_destructive_powershell_cmdlets() {
        for cmd in [
            "powershell -Command \"Format-Volume C\"",
            "Stop-Computer -Force",
            "Clear-Content C:\\important.txt",
            "Invoke-Expression $cmd",
        ] {
            assert!(
                check_dangerous(cmd).is_some(),
                "高危 cmdlet 必须被拦下: {cmd}"
            );
        }
    }

    #[test]
    fn truncate_keeps_head_and_tail() {
        let long = "x".repeat(MAX_OUTPUT * 2);
        let (out, truncated) = truncate_output(&long, MAX_OUTPUT);
        assert!(truncated);
        assert!(out.len() <= MAX_OUTPUT + 40);
        assert!(out.contains("中间省略"));

        // 未超长时原样返回
        let (out, truncated) = truncate_output("短输出", MAX_OUTPUT);
        assert!(!truncated);
        assert_eq!(out, "短输出");
    }

    #[test]
    fn truncate_is_utf8_safe() {
        let long = "中".repeat(MAX_OUTPUT * 2);
        let (out, truncated) = truncate_output(&long, MAX_OUTPUT);
        assert!(truncated);
        // 必须是合法 UTF-8（Rust String 已保证，关键是不 panic）
        assert!(out.contains('中'));
        assert!(out.chars().count() <= MAX_OUTPUT + 20);
    }

    #[test]
    fn first_line_caps_ui_text() {
        assert_eq!(first_line("cargo build"), "cargo build");
        let long = "echo ".to_string() + &"a".repeat(200);
        let out = first_line(&long);
        assert!(out.chars().count() <= 61);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn decode_handles_both_utf8_and_gbk() {
        // UTF-8 原样通过
        let utf8 = "测试中文 abc";
        assert!(decode_bytes(utf8.as_bytes()).contains("测试中文"));

        // GBK 字节应被正确还原为中文（而非乱码）
        let (gbk, _, _) = encoding_rs::GBK.encode("测试中文");
        let decoded = decode_bytes(&gbk);
        assert!(
            decoded.contains("测试中文"),
            "GBK 解码失败: {:?}",
            decoded
        );
    }

    #[test]
    fn quotes_and_pipes_survive_transport() {
        // 回归：Rust 的 Command::arg 会把 `"` 转义成 `\"`，cmd 无法识别，
        // 导致 `git commit -m "..."` 这类命令必然失败。改用环境变量传递后修复。
        let ws = std::env::temp_dir().join("aura_exec_quote");
        let _ = std::fs::create_dir_all(&ws);

        // 引号内容需原样保留
        let out = run_script(
            &ws,
            &serde_json::json!({ "command": "echo \"Add MST code\" done" }),
        )
        .unwrap();
        assert!(
            out.result.contains("Add MST code"),
            "引号内容未正确传递: {:?}",
            out.result
        );

        // 管道等复合命令需可用
        let out = run_script(
            &ws,
            &serde_json::json!({ "command": "echo hello | findstr h" }),
        )
        .unwrap();
        assert!(out.result.contains("hello"), "管道失效: {:?}", out.result);

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn chinese_output_survives_encoding() {
        // 回归：cmd 默认按 GBK 代码页输出，Rust 按 UTF-8 解码会得到乱码。
        // build_command 前置 chcp 65001 后中文必须原样回填给模型。
        let ws = std::env::temp_dir().join("aura_exec_cn");
        let _ = std::fs::create_dir_all(&ws);
        let out = run_script(&ws, &serde_json::json!({ "command": "echo 测试中文 abc" })).unwrap();
        assert!(
            out.result.contains("测试中文"),
            "中文未正确传递/解码: {:?}",
            out.result
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn runs_simple_command_in_workspace() {
        let ws = std::env::temp_dir().join("aura_exec_test");
        let _ = std::fs::create_dir_all(&ws);
        let out = run_script(&ws, &serde_json::json!({ "command": "echo hello" })).unwrap();
        assert_eq!(out.label, "Exec 'echo hello'");
        assert!(out.result.contains("hello"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn rejects_dangerous_at_runtime() {
        let ws = std::env::temp_dir().join("aura_exec_reject");
        let _ = std::fs::create_dir_all(&ws);
        let err = run_script(&ws, &serde_json::json!({ "command": "rm -rf /" })).unwrap_err();
        assert!(err.contains("拦截"), "err: {}", err);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn requires_command_param() {
        let ws = std::env::temp_dir().join("aura_exec_param");
        let _ = std::fs::create_dir_all(&ws);
        let err = run_script(&ws, &serde_json::json!({})).unwrap_err();
        assert!(err.contains("command"), "err: {}", err);
        let err = run_script(&ws, &serde_json::json!({ "command": "   " })).unwrap_err();
        assert!(err.contains("不能为空"), "err: {}", err);
        let _ = std::fs::remove_dir_all(&ws);
    }
}