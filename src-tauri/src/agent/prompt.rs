use crate::agent::mode::Mode;
use crate::tools::{registry, ToolKind};

pub fn build_system_prompt(workspace: &str, global_rules: &str, mode: Mode) -> String {
    let mut p = String::new();
    p.push_str(
        "你是 Aura，一个在用户本地工作区中运行的 AI Agent。\
         你可以自主使用工具探索工作区、读取文件、检索内容，最终为用户解决问题。\n\
         当前工作区根目录：",
    );
    p.push_str(workspace);
    p.push('\n');

    // 模式纪律段前置：让模型先知道「现在是规划还是执行」，再读工具清单
    let discipline = mode_section(mode);
    if !discipline.is_empty() {
        p.push_str(&discipline);
    }

    // 工具清单由注册表生成并按模式过滤，新增工具无需改动此处
    p.push_str(&registry::prompt_section(mode.kinds()));

    p.push_str(
        "\n# 输出规则\n\
         - 可以先输出普通文本作为思考过程，无需任何标签。\n\
         - 一轮内可以给出多个工具调用，它们会**按你写出的顺序依次执行**，\
         执行完后一次性收到全部结果。\n\
         - 存在前后依赖的调用（例如先改文件再运行构建）也可以放在同一轮，\
         无需自行分批——顺序执行天然保证了依赖。\n\
         - 每个调用单独写成 <tool>{\"name\": ..., \"args\": {...}}</tool>，可连续给出多个。\n\
         - 每条工具结果都会带上 name 与 target，据此判断是哪个调用的返回。\n\
         - 任务完成后输出 <answer>最终答案</answer>。一旦输出 <answer>，任务即结束；\
         答案应完整、可直接使用，并用 Markdown 排版。\n\
         - 即使没有调用任何工具，最终回答也必须用 <answer></answer> 包裹，\
         禁止直接输出裸文本作为回答。\n",
    );

    // 写入类工具的使用纪律：仅在本模式确实放行 Write 工具时注入。
    // 判据用 mode 而非「注册表里是否存在」，否则 Plan 模式的 prompt 里
    // 会出现 write_file 等名字——与 Rust 侧闸门矛盾，徒增 Token 并误导模型。
    if mode.allows("write_file") && !registry::names_by_kind(ToolKind::Write).is_empty() {
        p.push_str(
            "\n# 修改文件\n\
             - 改动已有文件前必须先 read_file 确认现有内容，禁止凭猜测直接覆写。\n\
             - 优先 string_replace 做局部修改（最省 Token）；\
             仅在新建文件或整体重写时才用 write_file，它会覆盖文件全部内容。\n\
             - 成块替换一段代码时用 replace_lines，new_content 必须是该区间的完整新文本。\n\
             - string_replace 的 old_string 必须与文件内容完全一致（含缩进与空格）；\
             报「未找到」时先重新 read_file 对照，不要反复重试同样的参数。\n\
             - 一次工具调用只改一个文件；需要改多个文件时，\
             写成多个工具调用依次给出，系统会按顺序执行。\n\
             - 写工具只会返回写入摘要而非文件内容；需要确认改对了什么，再 read_file 查看。\n",
        );
    }

    // 命令执行纪律：同样仅在本模式放行 Exec 工具时注入
    if mode.allows("run_script") && !registry::names_by_kind(ToolKind::Exec).is_empty() {
        p.push_str(
            "\n# 执行命令\n\
             - run_script 用于验证修改结果（编译、跑测试、查看 git diff），不要用它做与当前任务无关的操作。\n\
             - 先看返回的**退出码**：非 0 表示命令失败，必须读懂输出并修复，不要在失败状态下继续往下走。\n\
             - 输出过长会被截断，只保留头尾；关键信息请改用更精确的命令重新获取。\n\
             - 命令失败后不要原样重试，先根据错误信息调整命令或修改代码。\n\
             - 提交信息含空格、括号或中文时，用文件传参而非引号：\
             先 write_file 写入 commit_msg.txt，再 git commit -F commit_msg.txt。\n",
        );
    }

p.push_str(&format!(
        "\n# 运行限制\n\
         - 路径用正斜杠 /，且必须位于工作区内；禁止 ..，越界会被直接拦截。\n\
         - 路径以 / 开头表示工作区根目录，例如 /src/main.rs 即工作区下的 src/main.rs；\
         列举根目录写 / 即可。\n\
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

/// 各模式的纪律段。Chat 返回空串——它就是纯对话，不额外加任何说明。
fn mode_section(mode: Mode) -> String {
    match mode {
        Mode::Chat => String::new(),
        Mode::Plan => "\n# 当前模式：规划（Plan）\n\
             - 本模式**只读**：不可修改任何文件，不可执行任何命令。工具清单里只有只读工具。\n\
             - 你的任务是先调研清楚，再产出可执行的计划，而不是动手改。\n\
             - 充分使用 list_files / read_file / search_workspace 了解现状后再规划，\
             不要凭空假设文件结构或内容。\n\
             - 计划输出格式（JSON 必须合法）：\n\
             <todo_plan>{\"todos\":[{\"id\":\"s1\",\"title\":\"步骤标题\"}]}</todo_plan>\n\
             - 步骤要求：3~8 条，每条只做一件事，id 用 s1、s2 递增；\
             title 要具体到可验证（如「新增 config.rs 并注册路由」），禁止「优化代码」这类空话。\n\
             - 输出 <todo_plan> 后，仍须用 <answer> 简要说明计划要点与关键取舍。\n\
             - 模式由用户决定，你不会自动进入执行模式。若用户要求你直接动手，\
             请在 <answer> 中说明：需要先切换到执行模式，你才能修改文件。\n"
            .to_string(),
        Mode::Build => "\n# 当前模式：执行（Build）\n\
             - 本模式允许写入文件与执行命令，按既定目标把事情做完。\n\
             - prompt 末尾的「当前任务清单」即为待办事项，请按顺序逐步推进，\
并在**开始或完成**某一步时更新其状态：\n\
             <todo_update>{\"id\":\"s1\",\"status\":\"in_progress\"}</todo_update>\n\
             - status 只取 pending / in_progress / done 三值；一次最多更新一个步骤。\n\
             - 不要跳过中间步骤直接宣布完成；每步做完应先验证（读文件或跑命令）再标记 done。\n\
             - 全部步骤 done 后，用 <answer> 汇总实际改动与验证结果。\n"
            .to_string(),
    }
}

/// 小模型在有清单时必须知道如何更新进度，否则清单永远停在初始状态。
///
/// 精简 prompt 原则上不带任何协议说明，但清单是**已经在进行中**的状态，
/// 缺了这条协议小模型会从头到尾不更新一次，面板就成了摆设。
/// 因此只在确有清单时追加，代价是几行 token。
pub fn todo_update_brief() -> &'static str {
    "\n更新任务进度：<todo_update>{\"id\":\"步骤id\",\"status\":\"in_progress\"}</todo_update>\
     （status 取 pending / in_progress / done，开始与完成某一步时各更新一次）\n"
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
         一轮内可连续给出多个工具调用，系统会按写出的顺序依次执行，\
         随后一次性返回全部结果。\n\
         每条结果都带 name，可据此判断是哪个调用的返回。\n\
         最终回答用 <answer>...</answer> 包裹。\n\
         路径用 /，以 / 开头表示工作区根目录，禁 ..。\n",
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
    fn build_prompt_includes_write_tools_and_discipline() {
        let p = build_system_prompt("/ws", "", Mode::Build);
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

    /// 只读两档的 prompt 都不得出现任何会改动工作区的工具名。
    ///
    /// Rust 闸门放行不了它们，prompt 却报有，会直接诱导模型去调一个
    /// 注定被拒的工具。
    #[test]
    fn read_only_prompts_contain_no_mutating_tool_names() {
        for mode in [Mode::Chat, Mode::Plan] {
            let p = build_system_prompt("/ws", "", mode);
            for t in registry::names_by_kind(ToolKind::Write)
                .into_iter()
                .chain(registry::names_by_kind(ToolKind::Exec))
            {
                assert!(!p.contains(t), "{} prompt 不应出现工具名 {t}", mode.as_str());
            }
            // 相应纪律段也一并消失
            assert!(!p.contains("# 修改文件"), "{}", mode.as_str());
            assert!(!p.contains("# 执行命令"), "{}", mode.as_str());
            // 只读工具必须在
            assert!(p.contains("read_file"), "{}", mode.as_str());
            assert!(p.contains("search_workspace"), "{}", mode.as_str());
        }
    }

    #[test]
    fn plan_prompt_explains_readonly_and_plan_protocol() {
        let p = build_system_prompt("/ws", "", Mode::Plan);
        assert!(p.contains("# 当前模式：规划"));
        assert!(p.contains("只读"));
        assert!(p.contains("<todo_plan>"));
        // 必须提醒仍需 <answer> 收尾，否则解析器等不到终止标签
        assert!(p.contains("<answer>"));
    }

    #[test]
    fn build_prompt_explains_todo_update_protocol() {
        let p = build_system_prompt("/ws", "", Mode::Build);
        assert!(p.contains("# 当前模式：执行"));
        assert!(p.contains("<todo_update>"));
        assert!(p.contains("in_progress"));
        assert!(p.contains("write_file"), "执行模式应放行写工具");
    }

    #[test]
    fn chat_prompt_has_no_mode_section() {
        // Chat 必须与 v0.3.x 完全一致，不引入任何新约束
        let p = build_system_prompt("/ws", "", Mode::Chat);
        assert!(!p.contains("# 当前模式"));
        assert!(!p.contains("<todo_plan>"));
        assert!(!p.contains("<todo_update>"));
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
    fn global_rules_are_appended_in_every_mode() {
        for mode in [Mode::Chat, Mode::Plan, Mode::Build] {
            let p = build_system_prompt("/ws", "始终使用中文注释", mode);
            assert!(p.contains("# 全局规则"), "{} 模式应带上全局规则", mode.as_str());
            assert!(p.contains("始终使用中文注释"));
        }
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
        // 与最大的一份（Build）比，精简版才有意义
        let full = build_system_prompt("/ws", "", Mode::Build);
        let compact = build_compact_system_prompt("/ws", "");
        assert!(compact.len() < full.len() / 2);
    }

    /// Chat 就是纯对话，不注入任何模式纪律段。
    ///
    /// 它的只读性由 Rust 闸门与工具清单保证，无需 prompt 再啰嗦一遍。
    #[test]
    fn chat_has_no_mode_section() {
        let p = build_system_prompt("/ws", "", Mode::Chat);
        assert!(!p.contains("# 当前模式"));
        assert!(!p.contains("<todo_plan>"));
        assert!(!p.contains("<todo_update>"));
    }

    /// Plan 相对 Chat 的唯一差别就是产出清单。
    #[test]
    fn plan_differs_from_chat_only_by_the_plan_protocol() {
        let chat = build_system_prompt("/ws", "", Mode::Chat);
        let plan = build_system_prompt("/ws", "", Mode::Plan);
        assert!(plan.contains("<todo_plan>"));
        assert!(!chat.contains("<todo_plan>"));
        // 工具清单必须完全一致：两档权限相同，prompt 也不该有出入
        assert_eq!(
            registry::prompt_section(Mode::Chat.kinds()),
            registry::prompt_section(Mode::Plan.kinds()),
            "Chat 与 Plan 的工具清单应完全一致"
        );
    }

    /// Plan 模式不得声称会自行切到执行模式——模式由用户决定。
    #[test]
    fn plan_does_not_promise_automatic_mode_switch() {
        let p = build_system_prompt("/ws", "", Mode::Plan);
        assert!(
            !p.contains("用户确认计划后会切换到执行模式"),
            "不存在自动切换，向模型传达该预期会让它在用户催促时越权"
        );
        // 反之必须把决定权交回用户
        assert!(p.contains("切换到执行模式"));
    }

    #[test]
    fn build_prompt_points_at_the_injected_todo_section() {
        // 清单是以「当前任务清单」段落注入的，措辞必须指向它，
        // 而不是指向已废弃的「上下文中已有计划（todo_plan）」
        let p = build_system_prompt("/ws", "", Mode::Build);
        assert!(p.contains("当前任务清单"));
        assert!(!p.contains("已有计划（todo_plan）"));
    }

    #[test]
    fn compact_prompt_alone_lacks_todo_protocol() {
        // 精简 prompt 的设计前提就是不带协议；这条断言把该前提固定下来，
        // 防止后续有人「顺手」往里加内容
        let p = build_compact_system_prompt("/ws", "");
        assert!(!p.contains("<todo_update>"));
    }

    #[test]
    fn todo_update_brief_is_minimal_but_actionable() {
        let b = todo_update_brief();
        assert!(b.contains("<todo_update>"));
        assert!(b.contains("in_progress"));
        // 三种状态必须齐全，否则模型不知道待执行怎么写
        for s in ["pending", "in_progress", "done"] {
            assert!(b.contains(s), "brief 应说明状态 {s}");
        }
        // 必须够短，短到不破坏精简 prompt 的初衷
        assert!(b.len() < 220, "brief 过长会拖垮小模型的上下文预算");
    }
}
