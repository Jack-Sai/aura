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

/// 危险命令黑名单（小写匹配）。命中即拒绝执行。
const DANGEROUS: &[(&str, &str)] = &[
    ("rm -rf /", "递归删除根目录"),
    ("rm -rf ~", "递归删除用户主目录"),
    ("rm -rf *", "递归删除当前目录全部内容"),
    ("rmdir /s", "Windows 递归删除目录"),
    ("del /f", "Windows 强制删除文件"),
    ("del /q", "Windows 静默删除文件"),
    ("format ", "磁盘格式化"),
    ("mkfs", "文件系统格式化"),
    ("diskpart", "磁盘分区操作"),
    ("shutdown", "系统关机"),
    ("halt", "系统停机"),
    ("poweroff", "系统断电"),
    ("reg delete", "删除注册表"),
    ("takeown", "夺取文件所有权"),
    ("icacls", "修改文件权限"),
    (":(){", "fork 炸弹"),
    ("dd if=", "裸设备写入"),
    ("chmod -r 777 /", "递归放开根目录权限"),
    ("chown -r", "递归变更属主"),
];

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
}

/// Windows 上 cmd 输出通常是 CP936(GBK) 而非 UTF-8，直接按 UTF-8 解析会产生
/// 替换字符，模型看到的是乱码。这里先按 UTF-8 尝试，失败再回退 GBK。
#[cfg(windows)]
fn decode_bytes(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => {
            // 无额外依赖，用 lossy 转换至少保证可读
            // （完整 GBK 支持需引入 encoding_rs，此处保持零依赖）
            String::from_utf8_lossy(raw).into_owned()
        }
    }
}

#[cfg(not(windows))]
fn decode_bytes(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
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

    // 轮询等待：既能超时中断，也能在等待期间响应取消（由上层 tokio select 处理）
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let partial = collect_output(child);
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

    let raw_output = collect_output(child);
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

#[cfg(windows)]
fn build_command(root: &Path, command: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("cmd");
    cmd.arg("/C").arg(command).current_dir(root);
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

/// 读取子进程的全部输出（stdout + stderr 合并）。
///
/// 必须在子进程**已退出**后调用，否则 `wait_with_output` 会再次等待。
fn collect_output(mut child: std::process::Child) -> String {
    let mut buf = String::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        let mut raw = Vec::new();
        let _ = out.read_to_end(&mut raw);
        let text = decode_bytes(&raw);
        if !text.trim().is_empty() {
            buf.push_str(&text);
        }
    }
    if let Some(mut err) = child.stderr.take() {
        use std::io::Read;
        let mut raw = Vec::new();
        let _ = err.read_to_end(&mut raw);
        let text = decode_bytes(&raw);
        if !text.trim().is_empty() {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(&text);
        }
    }
    buf
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
            "rm -rf node_modules",
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

    #[test]
    fn conservative_about_dangerous_substrings() {
        // 黑名单是「宁可误拦」策略：即使危险词出现在引号里（shell 仍可能展开），
        // 也一律拦截。误拦的代价远小于漏拦。
        assert!(check_dangerous("echo 'rm -rf /'").is_some());
        assert!(check_dangerous("echo 'mkfs'").is_some());
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