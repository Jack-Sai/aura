pub fn build_system_prompt(workspace: &str, global_rules: &str) -> String {
    let mut p = String::new();
    p.push_str(
        "你是 Aura，一个在用户本地工作区中运行的 AI Agent。\
         你可以自主使用工具探索工作区、读取文件、检索内容，最终为用户解决问题。\n\
         当前工作区根目录：",
    );
    p.push_str(workspace);
    p.push('\n');

    p.push_str(
        "\n# 可用工具\n\
         需要调用工具时，输出如下格式的 JSON（必须同时包含 name 与 args，JSON 必须合法）：\n\
         <tool>{\"name\": \"工具名\", \"args\": {参数}}</tool>\n\n\
         1. list_files：单层列举目录，不递归。\n\
            {\"path\": \"src/\"}（工作区根目录写 /）\n\
            返回该层的文件与子目录列表，子目录以 / 结尾。\n\
         2. read_file：读取文本文件的指定行区间，返回带行号的内容。\n\
            {\"path\": \"src/main.rs\", \"start_line\": 1, \"end_line\": \"end\"}（end_line 也可为数字）\n\
         3. search_workspace：在工作区全文检索，返回带文件路径和行号的片段。\n\
            {\"query\": \"关键词\"}\n",
    );

    p.push_str(
        "\n# 输出规则\n\
         - 可以先输出普通文本作为思考过程，无需任何标签。\n\
         - 一次只调用一个工具；收到工具结果后继续思考或作答。\n\
         - 任务完成后输出 <answer>最终答案</answer>。一旦输出 <answer>，任务即结束；\
         答案应完整、可直接使用，并用 Markdown 排版。\n\
         - 即使没有调用任何工具，最终回答也必须用 <answer></answer> 包裹，\
         禁止直接输出裸文本作为回答。\n",
    );

    p.push_str(
        "\n# 运行限制\n\
         - 所有路径必须使用正斜杠 /，且必须位于工作区内；禁止绝对路径、禁止 ..，越界会被直接拦截。\n\
         - 同一个工具连续调用 5 次后，第 6 次会被拦截并返回报错；\
         此时请更换工具，或基于已有信息直接回答。\n\
         - 文件较大或不确定位置时，优先用 search_workspace 定位，再用 read_file 精读。\n\
         - 工具结果以 <tool_result> 形式返回；若以 Error: 开头说明失败，\
         读懂错误后调整参数重试，不要重复同样的调用。\n",
    );

    let rules = global_rules.trim();
    if !rules.is_empty() {
        p.push_str("\n# 全局规则\n");
        p.push_str(rules);
        p.push('\n');
    }

    p
}

/// 精简版 System Prompt（用于小参数本地模型，减少 Token 消耗与认知负荷）。
pub fn build_compact_system_prompt(workspace: &str, global_rules: &str) -> String {
    let mut p = String::new();
    p.push_str(
        "你是 Aura，在本地工作区运行的 AI Agent。可用工具：list_files、read_file、search_workspace。\n\
         工作区：",
    );
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
