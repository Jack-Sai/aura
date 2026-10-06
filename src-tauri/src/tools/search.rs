use std::fs;
use std::path::Path;

use serde_json::Value;

use super::ToolOutput;

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
];
const MAX_FILES: usize = 5000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const TOP_K: usize = 5;
const CONTEXT_LINES: usize = 1;
const MAX_OUTPUT: usize = 6000;

struct Hit {
    path: String,
    line: usize,
    score: u32,
}

struct Segment {
    path: String,
    start: usize,
    end: usize,
    body: String,
}

pub fn search_workspace(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .unwrap_or("");
    if query.is_empty() {
        return Err("缺少参数 `query`".into());
    }
    let root_c = root
        .canonicalize()
        .map_err(|e| format!("工作区不可用: {}", e))?;
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    let mut hits = collect_hits(&root_c, &terms);
    let label = format!("Search '{}'", query);
    if hits.is_empty() {
        return Ok(ToolOutput {
            label,
            result: "未找到匹配内容，可尝试更换关键词，或先用 list_files 查看目录结构。".into(),
        });
    }
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });

    let mut segments: Vec<Segment> = Vec::new();
    let mut budget = 0usize;
    'outer: for hit in &hits {
        if segments.len() >= TOP_K {
            break;
        }
        let idx0 = hit.line - 1;
        let start = idx0.saturating_sub(CONTEXT_LINES);
        let end = idx0 + CONTEXT_LINES;
        if segments.iter().any(|s| {
            s.path == hit.path && !(end < s.start || start > s.end)
        }) {
            continue;
        }
        let Ok(content) = fs::read_to_string(root_c.join(&hit.path)) else {
            continue;
        };
        let all: Vec<&str> = content.lines().collect();
        let end = end.min(all.len().saturating_sub(1));
        if start >= all.len() || start > end {
            continue;
        }
        let mut body = String::new();
        for i in start..=end {
            body.push_str(&format!("{}: {}\n", i + 1, all[i]));
        }
        let header = format!("{}:{}-{}\n", hit.path, start + 1, end + 1);
        if budget + header.len() + body.len() > MAX_OUTPUT {
            break 'outer;
        }
        budget += header.len() + body.len();
        segments.push(Segment {
            path: hit.path.clone(),
            start,
            end,
            body,
        });
    }

    let mut result = String::new();
    for seg in &segments {
        result.push_str(&format!("{}:{}-{}\n", seg.path, seg.start + 1, seg.end + 1));
        result.push_str(&seg.body);
        result.push('\n');
    }
    result.push_str("(可对上述文件使用 read_file 查看完整上下文)");
    Ok(ToolOutput { label, result })
}

fn collect_hits(root_c: &Path, terms: &[String]) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut scanned = 0usize;
    let mut queue = vec![root_c.to_path_buf()];
    while let Some(dir) = queue.pop() {
        if scanned >= MAX_FILES {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if scanned >= MAX_FILES {
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    queue.push(path);
                }
                continue;
            }
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            scanned += 1;
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let rel = path
                .strip_prefix(root_c)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            for (idx, line) in content.lines().enumerate() {
                let lower = line.to_lowercase();
                let score = terms.iter().filter(|t| lower.contains(t.as_str())).count() as u32;
                if score > 0 {
                    hits.push(Hit {
                        path: rel.clone(),
                        line: idx + 1,
                        score,
                    });
                }
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_matches_with_path_and_line() {
        let root = std::env::temp_dir().join("aura_search_test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        fs::write(root.join("src").join("db.rs"), "fn init_db() {}\nlet conn = open();\n").unwrap();
        fs::write(root.join("node_modules").join("pkg").join("index.js"), "init_db\n").unwrap();

        let out = search_workspace(&root, &json!({ "query": "init_db" })).unwrap();
        assert_eq!(out.label, "Search 'init_db'");
        assert!(out.result.contains("src/db.rs:1-2"), "result: {}", out.result);
        assert!(!out.result.contains("node_modules"), "应跳过 node_modules: {}", out.result);

        let empty = search_workspace(&root, &json!({ "query": "不存在的词xyz" })).unwrap();
        assert!(empty.result.contains("未找到匹配内容"));

        assert!(search_workspace(&root, &json!({})).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
