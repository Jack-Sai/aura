pub mod exec;
pub mod fs_ops;
pub mod registry;
pub mod search;
pub mod write_ops;

use std::path::{Path, PathBuf};

use serde_json::Value;

pub use registry::{is_known, ToolKind};

#[derive(Debug)]
pub struct ToolOutput {
    pub label: String,
    pub result: String,
}

#[derive(Debug)]
pub struct ToolFail {
    pub label: String,
    pub error: String,
}

/// 生成 UI 动作文案。工具列表见 `registry::TOOLS`。
pub fn tool_label(name: &str, args: &Value) -> String {
    registry::find(name)
        .map(|t| (t.label)(args))
        .unwrap_or_else(|| name.to_string())
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

/// 写入场景的路径解析：目标文件/目录可能**尚不存在**。
///
/// `ensure_inside` 依赖 `canonicalize()`，对不存在的路径直接失败，
/// 因此无法用于 `write_file` 这类创建新文件的工具。这里改为：
/// 1. 复用 [`resolve_path`] 的字符串级拦截（禁 `..`、绝对路径、反斜杠）
/// 2. 逐段向上回溯，找到最深的存在祖先
/// 3. 对该祖先做 `canonicalize` + `starts_with` 校验——这一步防住
///    「工作区内某符号链接指向外部目录，再在其下新建文件」的逃逸
/// 4. 把剩余段按原顺序拼回
///
/// 返回的路径未做 `canonicalize`，调用方需自行确保父目录已创建。
pub fn resolve_path_for_write(root: &Path, path: &str) -> Result<PathBuf, String> {
    let resolved = resolve_path(root, path)?;
    let root_c = root
        .canonicalize()
        .map_err(|e| format!("工作区不可用: {}", e))?;

    // 已是工作区根或其现存子路径时，直接复用常规校验
    if resolved.exists() {
        return ensure_inside(root, &resolved);
    }

    // 自下而上剥离不存在的段，记录待拼回的后缀
    let mut existing = resolved.clone();
    let mut trailing: Vec<std::ffi::OsString> = Vec::new();
    loop {
        let Some(name) = existing.file_name().map(|s| s.to_os_string()) else {
            break;
        };
        let Some(parent) = existing.parent().map(|p| p.to_path_buf()) else {
            break;
        };
        trailing.push(name);
        existing = parent;
        if existing.exists() {
            break;
        }
    }

    let existing_c = existing
        .canonicalize()
        .map_err(|e| format!("路径不可访问: {}", e))?;
    if !existing_c.starts_with(&root_c) {
        return Err("路径越出工作区沙盒，已拦截".into());
    }

    let mut final_path = existing_c;
    for seg in trailing.iter().rev() {
        final_path.push(seg);
    }
    Ok(final_path)
}

/// 原子写入：先写同目录临时文件，再 `rename` 覆盖目标。
///
/// 同目录保证 rename 在同一文件系统内是原子的；直接写目标文件的风险是
/// 写入中途崩溃/磁盘满会留下半截文件，rename 则要么完整替换要么保持原样。
pub fn atomic_write(target: &Path, content: &str) -> Result<(), String> {
    let dir = target
        .parent()
        .ok_or_else(|| "目标路径缺少父目录".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {}", e))?;

    let file_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("tmp");
    // 进程内唯一，避免并发写入同一目录时互相覆盖临时文件
    let tmp = dir.join(format!(
        ".aura-{}-{}.tmp",
        file_name,
        std::process::id()
    ));

    let write_result = std::fs::write(&tmp, content);
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("写入失败: {}", e));
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换目标文件失败: {}", e));
    }
    Ok(())
}

pub(crate) fn display_path(path: &str) -> String {
    let p = path.trim().trim_start_matches("./");
    if p.is_empty() || p == "." {
        "/".to_string()
    } else {
        p.trim_end_matches('/').to_string()
    }
}

/// 异步执行工具。
///
/// 工具本体仍是同步函数（文件 IO 不值得引入异步复杂度），但通过
/// `spawn_blocking` 移交阻塞线程池执行：`run_script` 这类要跑子进程的
/// 工具若直接同步调用，会阻塞 tokio worker，导致 SSE 流式输出中断、
/// 界面假死、停止按钮失灵。
pub async fn execute(
    workspace: std::path::PathBuf,
    name: String,
    args: Value,
) -> Result<ToolOutput, ToolFail> {
    let label = tool_label(&name, &args);
    let result = match registry::find(&name) {
        Some(spec) => {
            let run = spec.run;
            match tokio::task::spawn_blocking(move || run(&workspace, &args)).await {
                Ok(r) => r,
                Err(e) => Err(format!("工具执行异常: {}", e)),
            }
        }
        None => Err(format!("未知工具: {}", name)),
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
    fn write_path_allows_new_files_and_missing_dirs() {
        let (ws, _outside, base) = sandbox_fixture("write_ok");
        // Windows 下 canonicalize 会加 `\\?\` 前缀，比较前统一剥离
        let norm = |p: &Path| -> String {
            p.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .replace('\\', "/")
        };
        let ws_norm = norm(&ws);

        // 新文件：父目录存在
        assert_eq!(norm(&resolve_path_for_write(&ws, "new.txt").unwrap()), format!("{}/new.txt", ws_norm));
        // 深层不存在的目录
        assert_eq!(
            norm(&resolve_path_for_write(&ws, "a/b/c/deep.txt").unwrap()),
            format!("{}/a/b/c/deep.txt", ws_norm)
        );
        // 已存在文件走常规校验
        let p = resolve_path_for_write(&ws, "inside.txt").unwrap();
        assert!(p.ends_with("inside.txt"));
        // 工作区根
        assert_eq!(
            norm(&resolve_path_for_write(&ws, "/").unwrap()),
            norm(&ws.canonicalize().unwrap())
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn write_path_rejects_traversal_and_absolute() {
        let (ws, _outside, base) = sandbox_fixture("write_reject");
        assert!(resolve_path_for_write(&ws, "../evil.txt").is_err());
        assert!(resolve_path_for_write(&ws, "a/../../evil.txt").is_err());
        assert!(resolve_path_for_write(&ws, "/etc/passwd").is_err());
        assert!(resolve_path_for_write(&ws, "C:/Windows/evil").is_err());
        assert!(resolve_path_for_write(&ws, "a\\b").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn write_path_blocks_symlink_escape_for_new_files() {
        use std::os::windows::fs::symlink_dir;
        let (ws, outside, base) = sandbox_fixture("write_symlink");
        let link = ws.join("escape_link");
        match symlink_dir(&outside, &link) {
            Ok(()) => {
                // 关键场景：链接指向工作区外，且目标文件尚不存在。
                // ensure_inside 在此处会因 canonicalize 失败而误判，
                // resolve_path_for_write 必须通过「最深存在祖先」正确拦截。
                let err = resolve_path_for_write(&ws, "escape_link/new.txt")
                    .expect_err("符号链接逃逸必须被拦截");
                assert!(
                    err.contains("沙盒") || err.contains("越出"),
                    "err: {}",
                    err
                );
                // 更深层同样拦截
                assert!(resolve_path_for_write(&ws, "escape_link/a/b/c.txt").is_err());
            }
            Err(e) => {
                eprintln!("skip symlink test (需要开发者模式或管理员权限): {}", e);
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn atomic_write_creates_file_and_replaces_content() {
        let (ws, _outside, base) = sandbox_fixture("atomic");
        let target = ws.join("out.txt");

        atomic_write(&target, "第一版").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "第一版");

        // 覆写
        atomic_write(&target, "第二版内容更长").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "第二版内容更长");

        // 不应残留临时文件
        let leftovers: Vec<_> = std::fs::read_dir(&ws)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".aura-"))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件: {:?}", leftovers);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn atomic_write_creates_missing_dirs() {
        let (ws, _outside, base) = sandbox_fixture("atomic_dirs");
        let target = ws.join("x").join("y").join("out.txt");
        atomic_write(&target, "内容").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "内容");
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
