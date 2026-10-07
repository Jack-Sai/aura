pub mod fs_ops;
pub mod search;

use std::path::{Path, PathBuf};

use serde_json::Value;

pub struct ToolOutput {
    pub label: String,
    pub result: String,
}

pub struct ToolFail {
    pub label: String,
    pub error: String,
}

pub fn tool_label(name: &str, args: &Value) -> String {
    match name {
        "list_files" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("/")
                .trim();
            if path == "/" || path.is_empty() {
                "Read /".to_string()
            } else {
                format!("Read {}/", display_path(path))
            }
        }
        "read_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let start = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1);
            let end = match args.get("end_line") {
                Some(Value::Number(n)) => n.to_string(),
                _ => "end".to_string(),
            };
            if start == 1 && end == "end" {
                format!("Read {}", display_path(path))
            } else {
                format!("Read {} L{} - L{}", display_path(path), start, end)
            }
        }
        "search_workspace" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            format!("Search '{}'", query)
        }
        other => other.to_string(),
    }
}

pub fn resolve_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("路径不能为空".into());
    }
    if path.contains('\\') {
        return Err("路径必须使用正斜杠 /，不能使用反斜杠".into());
    }
    if path == "/" {
        return Ok(root.to_path_buf());
    }
    if path.starts_with('/') {
        return Err("禁止绝对路径，只能使用相对工作区根目录的路径（根目录用 /）".into());
    }
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return Err("禁止绝对路径，只能使用相对工作区根目录的路径".into());
    }
    let mut resolved = root.to_path_buf();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Err("禁止使用 ..，不能越出工作区".into()),
            s => resolved.push(s),
        }
    }
    Ok(resolved)
}

pub fn ensure_inside(root: &Path, target: &Path) -> Result<PathBuf, String> {
    let root_c = root
        .canonicalize()
        .map_err(|e| format!("工作区不可用: {}", e))?;
    let target_c = target
        .canonicalize()
        .map_err(|e| format!("路径不存在或无法访问: {}", e))?;
    if !target_c.starts_with(&root_c) {
        return Err("路径越出工作区沙盒，已拦截".into());
    }
    Ok(target_c)
}

fn display_path(path: &str) -> String {
    let p = path.trim().trim_start_matches("./");
    if p.is_empty() || p == "." {
        "/".to_string()
    } else {
        p.trim_end_matches('/').to_string()
    }
}

pub fn execute(workspace: &Path, name: &str, args: &Value) -> Result<ToolOutput, ToolFail> {
    let label = tool_label(name, args);
    let result = match name {
        "list_files" => fs_ops::list_files(workspace, args),
        "read_file" => fs_ops::read_file(workspace, args),
        "search_workspace" => search::search_workspace(workspace, args),
        other => Err(format!("未知工具: {}", other)),
    };
    result.map_err(|error| ToolFail { label, error })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        std::env::temp_dir().join("aura_tools_test_root")
    }

    #[test]
    fn root_slash_maps_to_workspace() {
        let p = resolve_path(&root(), "/").unwrap();
        assert_eq!(p, root());
    }

    #[test]
    fn relative_path_resolves() {
        let p = resolve_path(&root(), "src/main.rs").unwrap();
        assert_eq!(p, root().join("src").join("main.rs"));
    }

    #[test]
    fn rejects_traversal_and_absolute() {
        assert!(resolve_path(&root(), "../etc").is_err());
        assert!(resolve_path(&root(), "a/../../b").is_err());
        assert!(resolve_path(&root(), "/etc/passwd").is_err());
        assert!(resolve_path(&root(), "C:/Windows").is_err());
        assert!(resolve_path(&root(), "a\\b").is_err());
        assert!(resolve_path(&root(), "").is_err());
    }

    #[test]
    fn rejects_more_traversal_variants() {
        // 纯 .. 与深层越界
        assert!(resolve_path(&root(), "..").is_err());
        assert!(resolve_path(&root(), "a/b/../../../c").is_err());
        assert!(resolve_path(&root(), "./../x").is_err());
        assert!(resolve_path(&root(), "a/./..").is_err());
        // 小写盘符
        assert!(resolve_path(&root(), "c:/windows").is_err());
        assert!(resolve_path(&root(), "e:aura").is_err());
        // UNC / 协议式绝对路径（前导斜杠或反斜杠）
        assert!(resolve_path(&root(), "//server/share").is_err());
        assert!(resolve_path(&root(), "\\\\server\\share").is_err());
        assert!(resolve_path(&root(), "file:///c:/x").is_ok()); // 相对路径，由 ensure_inside 兜底
    }

    #[test]
    fn accepts_normal_relative_variants() {
        // 空段与 . 段应被规范化，不构成越界
        let a = resolve_path(&root(), "a//b").unwrap();
        let b = resolve_path(&root(), "a/b").unwrap();
        assert_eq!(a, b);
        let c = resolve_path(&root(), "./src/./main.rs").unwrap();
        assert_eq!(c, root().join("src").join("main.rs"));
    }

    fn sandbox_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("aura_sandbox_{}", name));
        let _ = std::fs::remove_dir_all(&base);
        let ws = base.join("ws");
        let outside = base.join("outside");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(ws.join("inside.txt"), "inside").unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        (ws, outside, base)
    }

    #[test]
    fn ensure_inside_allows_root_and_inside_files() {
        let (ws, _outside, base) = sandbox_fixture("allow");
        assert!(ensure_inside(&ws, &ws).is_ok());
        assert!(ensure_inside(&ws, &ws.join("inside.txt")).is_ok());
        // 大小写混拼：canonicalize 统一为磁盘真实大小写
        let upper = PathBuf::from(ws.to_string_lossy().to_uppercase());
        assert!(ensure_inside(&ws, &upper).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ensure_inside_blocks_outside_paths() {
        let (ws, outside, base) = sandbox_fixture("outside");
        assert!(ensure_inside(&ws, &outside).is_err());
        assert!(ensure_inside(&ws, &outside.join("secret.txt")).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ensure_inside_blocks_symlink_escape() {
        use std::os::windows::fs::symlink_dir;
        let (ws, outside, base) = sandbox_fixture("symlink");
        let link = ws.join("escape_link");
        match symlink_dir(&outside, &link) {
            Ok(()) => {
                // 字符串解析层面仍在沙盒内，canonicalize 解析真实目标后必须拦截
                let resolved = resolve_path(&ws, "escape_link").unwrap();
                let err = ensure_inside(&ws, &resolved).unwrap_err();
                assert!(err.contains("沙盒"), "err: {}", err);
                // 工具层同样拦截
                let args = serde_json::json!({ "path": "escape_link" });
                let err = fs_ops::list_files(&ws, &args)
                    .err()
                    .expect("list_files 必须拦截符号链接逃逸");
                assert!(err.contains("沙盒") || err.contains("越出"), "err: {}", err);
            }
            Err(e) => {
                eprintln!("skip symlink test (需要开发者模式或管理员权限): {}", e);
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
