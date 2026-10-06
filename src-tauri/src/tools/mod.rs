pub mod fs_ops;
pub mod search;

use std::path::{Path, PathBuf};

use serde_json::Value;

pub struct ToolOutput {
    pub label: String,
    pub result: String,
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

pub fn execute(workspace: &Path, name: &str, args: &Value) -> Result<ToolOutput, String> {
    match name {
        "list_files" => fs_ops::list_files(workspace, args),
        "read_file" => fs_ops::read_file(workspace, args),
        "search_workspace" => search::search_workspace(workspace, args),
        other => Err(format!("未知工具: {}", other)),
    }
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
}
