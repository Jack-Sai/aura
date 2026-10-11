use crate::tools::{registry, ToolKind};

pub fn build_system_prompt(workspace: &str, global_rules: &str) -> String {
    let mut p = String::new();
    p.push_str(
        "你是 Aura，一个在用户本地工作区中运行的 AI Agent。\
         你可以自主使用工具探索工作区、读取文件、检索内容，最终为用户解决问题。\n\
         当前工作区根目录：",
    );
    p.push_str(workspace);
    p.push('\n');

    // 工具清单由注册表生成，新增工具无需改动此处
    p.push_str(&registry::prompt_section(&[
        ToolKind::Read,
        ToolKind::Write,
        ToolKind::Exec,
    ]));

    p.push_str(
        "\n# 输出规则\n\
         - 可以先输出普通文本作为思考过程，无需任何标签。\n\
         - 一次只调用一个工具；收到工具结果后继续思考或作答。\n\
         - 任务完成后输出 <answer>最终答案</answer>。一旦输出 <answer>，任务即结束；\
         答案应完整、可直接使用，并用 Markdown 排版。\n\
         - 即使没有调用任何工具，最终回答也必须用 <answer></answer> 包裹，\
         禁止直接输出裸文本作为回答。\n",
    );

    // 写入类工具的使用纪律：只在存在 Write 类工具时注入
    if !registry::names_by_kind(ToolKind::Write).is_empty() {
        p.push_str(
            "\n# 修改文件\n\
             - 改动已有文件前必须先 read_file 确认现有内容，禁止凭猜测直接覆写。\n\
             - 优先 string_replace 做局部修改（最省 Token）；\
             仅在新建文件或整体重写时才用 write_file，它会覆盖文件全部内容。\n\
             - 成块替换一段代码时用 replace_lines，new_content 必须是该区间的完整新文本。\n\
             - string_replace 的 old_string 必须与文件内容完全一致（含缩进与空格）；\
             报「未找到」时先重新 read_file 对照，不要反复重试同样的参数。\n\
             - 一次工具调用只改一个文件，改完确认结果再改下一个。\n\
             - 写工具只会返回写入摘要而非文件内容；需要确认改对了什么，再 read_file 查看。\n",
        );
    }

    // 命令执行纪律：仅在存在 Exec 类工具时注入
    if !registry::names_by_kind(ToolKind::Exec).is_empty() {
        p.push_str(
            "\n# 执行命令\n\
             - run_script 用于验证修改结果（编译、跑测试、查看 git diff），不要用它做与当前任务无关的操作。\n\
             - 先看返回的**退出码**：非 0 表示命令失败，必须读懂输出并修复，不要在失败状态下继续往下走。\n\
             - 输出过长会被截断，只保留头尾；关键信息请改用更精确的命令重新获取。\n\
             - 命令失败后不要原样重试，先根据错误信息调整命令或修改代码。\n\
             - 提交信息含空格、括号或中文时，用文件传参而非引号：\n\
             先 write_file 写入 commit_msg.txt，再 git commit -F commit_msg.txt。\n",
        );
    }

    p.push_str(&format!(
        "\n# 运行限制\n\
         - 所有路径必须使用正斜杠 /，且必须位于工作区内；禁止绝对路径、禁止 ..，越界会被直接拦截。\n\
         - 同一个工具对**同一个目标**连续调用 {} 次后会被拦截并返回报错；\
         此时请更换工具或更换目标，或基于已有信息直接回答。\n\
         - 文件较大或不确定位置时，优先用 search_workspace 定位，再用 read_file 精读。\n\
         - 工具结果以 <tool_result> 形式返回；若以 Error: 开头说明失败，\
         读懂错误后调整参数重试，不要重复同样的调用。\n",
        registry::max_consecutive("read_file")
    ));

    let rules = global_rules.trim();
    if !rules.is_empty() {
        p.push_str("\n# 全局规则\n");
        p.push_str(rules);
        p.push('\n');
    }

    p
}

/// 精简版 System Prompt（用于小参数本地模型，减少 Token 消耗与认知负荷）。
///
/// 只注入只读工具：小模型难以稳定执行多轮写入，强行注入反而更易触发解析重试耗尽。
pub fn build_compact_system_prompt(workspace: &str, global_rules: &str) -> String {
    let read_tools = registry::names_by_kind(ToolKind::Read).join("、");
    let mut p = String::new();
    p.push_str(&format!(
        "你是 Aura，在本地工作区运行的 AI Agent。可用工具：{}。\n工作区：",
        read_tools
    ));
    p.push_str(workspace);
    p.push('\n');

    p.push_str(
        "工具调用格式：<tool>{\"name\": \"工具名\", \"args\": {...}}</tool>\n\
         最终回答用 <answer>...</answer> 包裹。\n\
         路径用 /，禁绝对路径与 ..。\n",
    );

    let rules = global_rules.trim();
    if !rules.is_empty() {
        p.push_str("\n全局规则：");
        p.push_str(rules);
        p.push('\n');
    }

    p
}

/// 根据模型 ID 判断是否为小参数模型（<= 7B 视为小模型）。
pub fn is_small_model(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();
    // 匹配如 0.6b, 1b, 3b, 7b, 8b 等；不匹配 70b, 72b 等
    if let Some(caps) = regex::Regex::new(r"(\d+(?:\.\d+)?)\s*[bB]").unwrap().captures(&lower) {
        if let Some(m) = caps.get(1) {
            if let Ok(v) = m.as_str().parse::<f32>() {
                return v <= 7.0;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_prompt_includes_write_tools_and_discipline() {
        let p = build_system_prompt("/ws", "");
        // 写工具已注入
        assert!(p.contains("write_file"));
        assert!(p.contains("string_replace"));
        assert!(p.contains("replace_lines"));
        // 写入纪律段
        assert!(p.contains("# 修改文件"));
        assert!(p.contains("read_file 确认现有内容"));
        // 命令执行工具与纪律段
        assert!(p.contains("run_script"));
        assert!(p.contains("# 执行命令"));
        assert!(p.contains("退出码"));
        // 连续调用上限来自注册表而非硬编码
        assert!(p.contains(&format!(
            "连续调用 {} 次",
            crate::tools::registry::max_consecutive("read_file")
        )));
    }

    #[test]
    fn compact_prompt_excludes_write_tools() {
        let p = build_compact_system_prompt("/ws", "");
        // 小模型只给只读工具，避免多轮写入导致解析重试耗尽
        assert!(!p.contains("write_file"));
        assert!(!p.contains("string_replace"));
        assert!(p.contains("read_file"));
        // 工具名由注册表生成
        assert!(p.contains("search_workspace"));
    }

    #[test]
    fn global_rules_are_appended() {
        let p = build_system_prompt("/ws", "始终使用中文注释");
        assert!(p.contains("# 全局规则"));
        assert!(p.contains("始终使用中文注释"));
    }

    #[test]
    fn is_small_model_classification() {
        assert!(is_small_model("qwen3:0.6b"));
        assert!(is_small_model("phi3:3.8b"));
        assert!(!is_small_model("gpt-4o"));
        assert!(!is_small_model("deepseek-v4-pro"));
    }

    #[test]
    fn small_model_detection() {
        assert!(is_small_model("qwen3:0.6b"));
        assert!(is_small_model("llama3.2:1b"));
        assert!(is_small_model("phi3:3.8b"));
        assert!(is_small_model("gemma2:7b"));
        assert!(is_small_model("model-7B"));
        assert!(!is_small_model("llama3:8b"));
        assert!(!is_small_model("nemotron-3.5-lightning"));
        assert!(!is_small_model("gpt-4o"));
        assert!(!is_small_model("model-70b"));
    }

    #[test]
    fn compact_prompt_shorter() {
        let full = build_system_prompt("/ws", "");
        let compact = build_compact_system_prompt("/ws", "");
        assert!(compact.len() < full.len() / 2);
    }
}
