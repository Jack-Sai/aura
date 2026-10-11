//! 文件写入类工具（v0.3.0）
//!
//! 三个工具共享同一条安全链路：
//! `resolve_path_for_write`（字符串级拦截 + 最深存在祖先的沙盒校验）
//! → 读取校验 → `atomic_write`（临时文件 + rename）。
//!
//! 回填一律只给**摘要**而非文件内容：模型自己刚写下的内容无需重复读回，
//! 全量回填会在多轮修改中迅速撑爆上下文。

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::{atomic_write, display_path, resolve_path_for_write, ToolOutput};

/// 单个工具回填的最大字符数（摘要本身很短，此处只是防御上限）。
const SUMMARY_CAP: usize = 200;

fn read_args<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("缺少参数 `{}` 或类型错误", key))
}

/// 读取待编辑文件，校验其为 UTF-8 文本。
fn read_for_edit(path: &Path, display: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("文件不存在: {}（write_file 可用于新建）", display)
        } else {
            format!("读取失败: {}（无法读取的文件请用 write_file 覆写）", e)
        }
    })
}

/// 生成写入摘要：`已写入 src/main.rs（128 行，3.4 KB）`
fn write_summary(display: &str, content: &str) -> String {
    let lines = content.lines().count().max(1);
    let bytes = content.len();
    let kb = bytes as f64 / 1024.0;
    let size = if kb >= 1.0 {
        format!("{:.1} KB", kb)
    } else {
        format!("{} B", bytes)
    };
    let s = format!("已写入 {}（{} 行，{}）", display, lines, size);
    // 按字符截断：直接用 len() 切会破坏中文字符的 UTF-8 边界
    if s.chars().count() > SUMMARY_CAP {
        let mut out: String = s.chars().take(SUMMARY_CAP).collect();
        out.push('…');
        out
    } else {
        s
    }
}

/// `write_file`：创建新文件或全量覆写，父目录不存在时递归创建。
pub fn write_file(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let path = read_args(args, "path")?;
    let content = read_args(args, "content")?;
    let target = resolve_path_for_write(root, path)?;
    let shown = display_path(path);

    if target.is_dir() {
        return Err(format!("目标是目录，无法写入: {}", shown));
    }
    // 已存在的文件走一次常规校验（防止对目录/链接写入）
    if target.exists() {
        super::ensure_inside(root, &target)?;
    }

    atomic_write(&target, content)?;
    Ok(ToolOutput {
        label: format!("Write {}", shown),
        result: write_summary(&shown, content),
    })
}

/// `string_replace`：基于字符串匹配的精准替换，最省 Token 的改法。
///
/// `old_string` 必须唯一命中，否则要求模型补充上下文——这能避免
/// 「替换了错误的位置」这类静默破坏。
pub fn string_replace(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let path = read_args(args, "path")?;
    let old = read_args(args, "old_string")?;
    let new = read_args(args, "new_string")?;
    let replace_all = args
        .get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let target = resolve_path_for_write(root, path)?;
    let shown = display_path(path);
    let content = read_for_edit(&target, &shown)?;

    if old.is_empty() {
        return Err("`old_string` 不能为空".into());
    }
    let hits = content.matches(old).count();
    if hits == 0 {
        // 回填一段上下文，帮助模型定位
        return Err(format!(
            "在 {} 中未找到 `old_string`（文件共 {} 行）。请先用 read_file 确认现有内容，\
             并确保 old_string 与文件中的文本完全一致（含缩进）。",
            shown,
            content.lines().count()
        ));
    }
    if hits > 1 && !replace_all {
        return Err(format!(
            "`old_string` 在 {} 中出现 {} 次，无法确定替换位置。\
             请补充更多上下文使其唯一，或显式传入 \"replace_all\": true。",
            shown, hits
        ));
    }

    let updated = if replace_all {
        content.replace(old, new)
    } else {
        content.replacen(old, new, 1)
    };
    if updated == content {
        return Err(format!("替换后内容无变化，请检查 old_string 与 new_string: {}", shown));
    }
    atomic_write(&target, &updated)?;
    let times = if replace_all { hits } else { 1 };
    Ok(ToolOutput {
        label: format!("Edit {}", shown),
        result: format!(
            "已在 {} 中完成 {} 处替换（{} → {} 行，{}）",
            shown,
            times,
            content.lines().count().max(1),
            updated.lines().count().max(1),
            if replace_all { "全部匹配" } else { "唯一匹配" }
        ),
    })
}

/// `replace_lines`：按行号区间整段替换，替换内容是**该区间的完整新文本**。
pub fn replace_lines(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let path = read_args(args, "path")?;
    let new_content = read_args(args, "new_content")?;
    let start = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .ok_or("缺少参数 `start_line` 或类型错误")? as usize;
    let end = match args.get("end_line") {
        Some(v) if v.is_string() => {
            let s = v.as_str().unwrap_or("");
            if s == "end" {
                usize::MAX
            } else {
                return Err("`end_line` 必须是整数或 \"end\"".into());
            }
        }
        Some(v) => v
            .as_u64()
            .ok_or("`end_line` 必须是整数或 \"end\"")? as usize,
        None => return Err("缺少参数 `end_line`（整数或 \"end\"）".into()),
    };
    if start < 1 {
        return Err("`start_line` 必须 >= 1".into());
    }
    if end != usize::MAX && end < start {
        return Err("`end_line` 不能小于 `start_line`".into());
    }

    let target = resolve_path_for_write(root, path)?;
    let shown = display_path(path);
    let content = read_for_edit(&target, &shown)?;

    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let total = lines.len();
    if total == 0 {
        return Err(format!("文件为空，无法按行号替换: {}", shown));
    }
    if start > total {
        return Err(format!(
            "`start_line` 超出文件范围：文件共 {} 行（{}）",
            total, shown
        ));
    }
    let end_clamped = if end == usize::MAX { total } else { end.min(total) };

    let mut replacement: Vec<String> = new_content.lines().map(|s| s.to_string()).collect();
    // 允许用空串清空该区间
    lines.splice(start - 1..end_clamped, replacement.drain(..));

    let joined = lines.join("\n");
    let trailing = if content.ends_with('\n') { "\n" } else { "" };
    atomic_write(&target, &format!("{}{}", joined, trailing))?;

    Ok(ToolOutput {
        label: format!("Edit {} L{} - L{}", shown, start, end_clamped),
        result: format!(
            "已替换 {} 的 L{} - L{}（文件现 {} 行）",
            shown,
            start,
            end_clamped,
            lines.len()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_fixture(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("aura_writeops_{}", name));
        let _ = fs::remove_dir_all(&base);
        let ws = base.join("ws");
        fs::create_dir_all(&ws).unwrap();
        (ws, base)
    }

    #[test]
    fn write_file_creates_and_overwrites() {
        let (ws, base) = ws_fixture("create");
        let out = write_file(&ws, &serde_json::json!({ "path": "a.txt", "content": "第一版" })).unwrap();
        assert_eq!(out.label, "Write a.txt");
        assert!(out.result.contains("已写入 a.txt"));
        assert_eq!(fs::read_to_string(ws.join("a.txt")).unwrap(), "第一版");
        // 回填必须是摘要而非内容
        assert!(!out.result.contains("第一版"));

        // 覆写
        write_file(&ws, &serde_json::json!({ "path": "a.txt", "content": "第二版" })).unwrap();
        assert_eq!(fs::read_to_string(ws.join("a.txt")).unwrap(), "第二版");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn write_file_creates_nested_dirs() {
        let (ws, base) = ws_fixture("nested");
        write_file(
            &ws,
            &serde_json::json!({ "path": "x/y/z/deep.txt", "content": "内容" }),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(ws.join("x/y/z/deep.txt")).unwrap(), "内容");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn write_file_rejects_traversal() {
        let (ws, base) = ws_fixture("reject");
        let err = write_file(&ws, &serde_json::json!({ "path": "../evil.txt", "content": "x" }))
            .unwrap_err();
        assert!(err.contains(".."), "err: {}", err);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn write_file_rejects_directory_target() {
        let (ws, base) = ws_fixture("dir");
        fs::create_dir_all(ws.join("adir")).unwrap();
        let err = write_file(&ws, &serde_json::json!({ "path": "adir", "content": "x" })).unwrap_err();
        assert!(err.contains("目录"), "err: {}", err);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn string_replace_requires_unique_match() {
        let (ws, base) = ws_fixture("unique");
        fs::write(ws.join("c.txt"), "a\nfoo\nb\nfoo\nc").unwrap();

        // 出现两次且未指定 replace_all → 拒绝
        let err = string_replace(
            &ws,
            &serde_json::json!({ "path": "c.txt", "old_string": "foo", "new_string": "bar" }),
        )
        .unwrap_err();
        assert!(err.contains("2 次"), "err: {}", err);

        // 显式 replace_all → 全部替换
        let out = string_replace(
            &ws,
            &serde_json::json!({
                "path": "c.txt", "old_string": "foo",
                "new_string": "bar", "replace_all": true
            }),
        )
        .unwrap();
        assert_eq!(out.label, "Edit c.txt");
        assert_eq!(fs::read_to_string(ws.join("c.txt")).unwrap(), "a\nbar\nb\nbar\nc");
        assert!(out.result.contains("2 处"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn string_replace_single_hit_and_missing() {
        let (ws, base) = ws_fixture("single");
        fs::write(ws.join("c.txt"), "line1\nline2\nline3").unwrap();

        let out = string_replace(
            &ws,
            &serde_json::json!({ "path": "c.txt", "old_string": "line2", "new_string": "改过的line2" }),
        )
        .unwrap();
        assert!(out.result.contains("1 处"));
        assert_eq!(
            fs::read_to_string(ws.join("c.txt")).unwrap(),
            "line1\n改过的line2\nline3"
        );

        // 未命中应报错并提示先读文件
        let err = string_replace(
            &ws,
            &serde_json::json!({ "path": "c.txt", "old_string": "不存在的", "new_string": "x" }),
        )
        .unwrap_err();
        assert!(err.contains("未找到"), "err: {}", err);
        assert!(err.contains("read_file"), "err: {}", err);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn string_replace_missing_file_hint() {
        let (ws, base) = ws_fixture("missing");
        let err = string_replace(
            &ws,
            &serde_json::json!({ "path": "nope.txt", "old_string": "a", "new_string": "b" }),
        )
        .unwrap_err();
        assert!(err.contains("write_file"), "err: {}", err);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn replace_lines_swaps_range() {
        let (ws, base) = ws_fixture("lines");
        fs::write(ws.join("l.txt"), "1\n2\n3\n4\n5").unwrap();

        let out = replace_lines(
            &ws,
            &serde_json::json!({
                "path": "l.txt", "start_line": 2, "end_line": 3,
                "new_content": "新A\n新B"
            }),
        )
        .unwrap();
        assert_eq!(out.label, "Edit l.txt L2 - L3");
        assert_eq!(fs::read_to_string(ws.join("l.txt")).unwrap(), "1\n新A\n新B\n4\n5");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn replace_lines_to_end_and_clamps() {
        let (ws, base) = ws_fixture("lines_end");
        fs::write(ws.join("l.txt"), "1\n2\n3").unwrap();

        replace_lines(
            &ws,
            &serde_json::json!({
                "path": "l.txt", "start_line": 2, "end_line": "end",
                "new_content": "X"
            }),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(ws.join("l.txt")).unwrap(), "1\nX");

        // end_line 超出范围应被截断而非报错
        replace_lines(
            &ws,
            &serde_json::json!({
                "path": "l.txt", "start_line": 1, "end_line": 999,
                "new_content": "ONLY"
            }),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(ws.join("l.txt")).unwrap(), "ONLY");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn replace_lines_validates_range() {
        let (ws, base) = ws_fixture("lines_bad");
        fs::write(ws.join("l.txt"), "1\n2\n3").unwrap();

        let err = replace_lines(
            &ws,
            &serde_json::json!({ "path": "l.txt", "start_line": 3, "end_line": 1, "new_content": "x" }),
        )
        .unwrap_err();
        assert!(err.contains("不能小于"), "err: {}", err);

        let err = replace_lines(
            &ws,
            &serde_json::json!({ "path": "l.txt", "start_line": 0, "end_line": 1, "new_content": "x" }),
        )
        .unwrap_err();
        assert!(err.contains(">= 1"), "err: {}", err);

        let err = replace_lines(
            &ws,
            &serde_json::json!({ "path": "l.txt", "start_line": 99, "end_line": 100, "new_content": "x" }),
        )
        .unwrap_err();
        assert!(err.contains("超出文件范围"), "err: {}", err);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn preserves_trailing_newline() {
        let (ws, base) = ws_fixture("eol");
        fs::write(ws.join("l.txt"), "1\n2\n3\n").unwrap();
        replace_lines(
            &ws,
            &serde_json::json!({ "path": "l.txt", "start_line": 1, "end_line": 1, "new_content": "一" }),
        )
        .unwrap();
        let content = fs::read_to_string(ws.join("l.txt")).unwrap();
        assert_eq!(content, "一\n2\n3\n");
        assert!(content.ends_with('\n'));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn summary_truncates_long_paths() {
        // 构造超过 SUMMARY_CAP 字符的展示名，验证按字符（而非字节）截断
        let long = "深".repeat(SUMMARY_CAP);
        let out = write_summary(&long, "内容");
        assert!(out.ends_with('…'), "超长应被截断并以省略号结尾");
        // 不能切坏 UTF-8：若此处 panic 即说明边界处理有误
        assert!(out.chars().count() <= SUMMARY_CAP + 1);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn summary_keeps_short_names_intact() {
        let out = write_summary("src/main.rs", "一行内容");
        assert!(out.contains("src/main.rs"));
        assert!(!out.contains('…'));
        assert!(out.contains("1 行"));
    }
}