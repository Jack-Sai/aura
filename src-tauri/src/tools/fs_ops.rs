use std::fs;
use std::path::Path;

use serde_json::Value;

use super::{display_path, ensure_inside, resolve_path, ToolOutput};

const MAX_LINES: usize = 500;

enum EndLine {
    ToEnd,
    Line(usize),
}

fn read_args<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("缺少参数 `{}` 或类型错误", key))
}

pub fn list_files(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let path = read_args(args, "path")?;
    let target = resolve_path(root, path)?;
    let target = ensure_inside(root, &target)?;
    let meta = fs::metadata(&target).map_err(|_| format!("目录不存在: {}", path))?;
    if !meta.is_dir() {
        return Err(format!("不是目录: {}", path));
    }
    let mut files: Vec<String> = Vec::new();
    for entry in fs::read_dir(&target).map_err(|e| format!("读取目录失败: {}", e))? {
        let entry = entry.map_err(|e| format!("读取目录失败: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or(false);
        if is_dir {
            files.push(format!("{}/", name));
        } else {
            files.push(name);
        }
    }
    files.sort();
    let shown = if path.trim() == "/" {
        "/".to_string()
    } else {
        let d = display_path(path);
        format!("{}/", d)
    };
    Ok(ToolOutput {
        label: format!("Read {}", shown),
        result: serde_json::json!({ "files": files }).to_string(),
    })
}

pub fn read_file(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let path = read_args(args, "path")?;
    let start = match args.get("start_line") {
        None => 1,
        Some(v) => v
            .as_u64()
            .ok_or("`start_line` 必须是整数".to_string())? as usize,
    };
    if start < 1 {
        return Err("`start_line` 必须 >= 1".into());
    }
    let end = match args.get("end_line") {
        None => EndLine::ToEnd,
        Some(Value::String(s)) if s == "end" => EndLine::ToEnd,
        Some(Value::Number(n)) => EndLine::Line(
            n.as_u64()
                .ok_or("`end_line` 必须是整数".to_string())? as usize,
        ),
        Some(_) => return Err("`end_line` 必须是整数或 \"end\"".into()),
    };
    if let EndLine::Line(n) = end {
        if n < start {
            return Err("`end_line` 不能小于 `start_line`".into());
        }
    }

    let target = resolve_path(root, path)?;
    let target = ensure_inside(root, &target)?;
    let meta = fs::metadata(&target).map_err(|_| format!("文件不存在: {}", path))?;
    if meta.is_dir() {
        return Err(format!("是目录，请使用 list_files: {}", path));
    }
    let content =
        fs::read_to_string(&target).map_err(|_| format!("无法读取为 UTF-8 文本: {}", path))?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    if start > total {
        return Err(format!("文件只有 {} 行", total));
    }
    let to = match end {
        EndLine::ToEnd => total,
        EndLine::Line(n) => n.min(total),
    };

    let mut out = String::new();
    let mut truncated = false;
    for (idx, line) in lines[start - 1..to].iter().enumerate() {
        if idx >= MAX_LINES {
            truncated = true;
            break;
        }
        out.push_str(&format!("{}: {}\n", start + idx, line));
    }
    if truncated {
        out.push_str("... (输出已截断，请缩小行范围，或改用 search_workspace)\n");
    }

    let label = match end {
        EndLine::ToEnd if start == 1 => format!("Read {}", display_path(path)),
        _ => {
            let end_num = match end {
                EndLine::Line(n) => n.min(total),
                EndLine::ToEnd => total,
            };
            format!("Read {} L{} - L{}", display_path(path), start, end_num)
        }
    };
    Ok(ToolOutput { label, result: out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("aura_fs_ops_{}", name));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("main.rs"), "fn main() {}\n// hi\nlet x = 1;\n").unwrap();
        fs::write(root.join("README.md"), "# Title\nbody\n").unwrap();
        root
    }

    #[test]
    fn lists_single_level_with_dirs() {
        let root = fixture("list");
        let out = list_files(&root, &json!({ "path": "/" })).unwrap();
        assert_eq!(out.label, "Read /");
        assert_eq!(out.result, json!({"files": ["README.md", "src/"]}).to_string());
        let out = list_files(&root, &json!({ "path": "src/" })).unwrap();
        assert_eq!(out.label, "Read src/");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_range_with_line_numbers() {
        let root = fixture("range");
        let out = read_file(&root, &json!({ "path": "src/main.rs", "start_line": 2, "end_line": 3 })).unwrap();
        assert_eq!(out.label, "Read src/main.rs L2 - L3");
        assert_eq!(out.result, "2: // hi\n3: let x = 1;\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_full_file_as_one_label() {
        let root = fixture("full");
        let out = read_file(&root, &json!({ "path": "README.md", "start_line": 1, "end_line": "end" })).unwrap();
        assert_eq!(out.label, "Read README.md");
        assert!(out.result.starts_with("1: # Title"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_bad_ranges_and_dirs() {
        let root = fixture("bad");
        assert!(read_file(&root, &json!({ "path": "src/main.rs", "start_line": 5, "end_line": "end" })).is_err());
        assert!(read_file(&root, &json!({ "path": "src/main.rs", "start_line": 3, "end_line": 1 })).is_err());
        assert!(read_file(&root, &json!({ "path": "src" })).is_err());
        assert!(read_file(&root, &json!({ "path": "../secret" })).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
