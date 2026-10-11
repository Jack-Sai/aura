//! 会话模式（v0.4.0）
//!
//! Chat / Plan / Build 三种模式决定两件事：
//!
//! 1. **哪些工具可用** —— Plan 只放行只读工具，禁止一切写入与命令执行。
//! 2. **System Prompt 的形态** —— 各模式注入不同的纪律段与输出协议。
//!
//! 关键约束：**模式隔离不能只靠 prompt**。模型受指令约束，但小模型与
//! 上下文被压缩时仍可能幻觉调用写工具。因此 [`Mode::allows`] 是 Rust 侧
//! 的硬闸门，命中即拒绝并回传可操作的错误文案，让模型改用只读方式继续。
//! prompt 负责「引导」，Rust 负责「兜底」，两者缺一不可。

use crate::tools::registry::{self, ToolKind};

/// 会话模式。由前端在每次发送消息时选定，可随时切换。
///
/// 三档权限是递进关系：对话 → 规划 → 执行。
///
/// - `Chat` 纯对话，只读。**刻意不注入任何模式纪律段**：
///   它是默认路径，额外说明只会稀释既有的输出规则。
/// - `Plan` = `Chat` + 产出任务清单，同样只读。
/// - `Build` 全工具，按清单推进。
///
/// 关键约束：**模式隔离不能只靠 prompt**。模型受指令约束，但小模型与
/// 上下文被压缩时仍可能幻觉调用写工具。因此 [`Mode::allows`] 是 Rust 侧
/// 的硬闸门，命中即拒绝并回传可操作的错误文案，让模型改用只读方式继续。
/// prompt 负责「引导」，Rust 负责「兜底」，两者缺一不可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 纯对话：只读，不加模式纪律段。
    #[default]
    Chat,
    /// 规划：`Chat` + 产出任务清单。只读。
    Plan,
    /// 执行：全工具，按清单推进。
    Build,
}

impl Mode {
    /// 解析前端传入的模式串。无法识别时回落 [`Mode::Chat`]。
    ///
    /// 回落而非报错是有意的：模式串来自前端，新增枚举值时旧版本后端不会
    /// 因解析失败而让整个对话不可用。
    pub fn parse(raw: &str) -> Mode {
        match raw.trim().to_ascii_lowercase().as_str() {
            "plan" => Mode::Plan,
            "build" => Mode::Build,
            _ => Mode::Chat,
        }
    }

    /// 稳定的英文标识，用于持久化与事件负载。
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Chat => "chat",
            Mode::Plan => "plan",
            Mode::Build => "build",
        }
    }

    /// 中文名，用于 System Prompt 与 UI 文案。
    pub fn zh(self) -> &'static str {
        match self {
            Mode::Chat => "对话",
            Mode::Plan => "规划",
            Mode::Build => "执行",
        }
    }

    /// 本模式允许的工具能力分类，用于 `prompt_section` 过滤。
    pub fn kinds(self) -> &'static [ToolKind] {
        match self {
            // 只读两档共用同一份白名单，对应 registry.rs 里 ToolKind 的原始设计意图。
            // Plan 与 Chat 的差异不在权限，而在是否产出清单。
            Mode::Chat | Mode::Plan => &[ToolKind::Read],
            Mode::Build => &[ToolKind::Read, ToolKind::Write, ToolKind::Exec],
        }
    }

    /// 本模式是否放行该工具。
    ///
    /// 仅对**已注册**的工具给出有意义的结论；未注册工具应交由调用方的
    /// 「unknown tool」分支处理，不要在这里混为一谈。
    pub fn allows(self, tool: &str) -> bool {
        match registry::find(tool) {
            Some(spec) => self.kinds().contains(&spec.kind),
            None => true,
        }
    }

    /// 本模式是否全工具。
    pub fn is_unrestricted(self) -> bool {
        self == Mode::Build
    }

    /// 构造「该模式下不可调用此工具」的错误文案。
    ///
    /// 必须给出**当前模式下真正可用的工具列表**，而不是笼统地说「不允许」——
    /// 模型收到回传后会据此改用正确工具，模糊的拒绝容易导致反复重试。
    pub fn reject_reason(self, tool: &str) -> String {
        let allowed = self
            .kinds()
            .iter()
            .flat_map(|k| registry::names_by_kind(*k))
            .collect::<Vec<_>>()
            .join("、");
        format!(
            "{} 模式为只读模式，不可调用 `{tool}`；本模式可用工具为：{allowed}。\
             如需修改文件或执行命令，请提示用户切换到执行模式。",
            self.zh()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recognizes_all_modes() {
        assert_eq!(Mode::parse("chat"), Mode::Chat);
        assert_eq!(Mode::parse("plan"), Mode::Plan);
        assert_eq!(Mode::parse("build"), Mode::Build);
    }

    #[test]
    fn parse_is_case_and_space_insensitive() {
        assert_eq!(Mode::parse("  PLAN "), Mode::Plan);
        assert_eq!(Mode::parse("Build"), Mode::Build);
    }

    #[test]
    fn parse_falls_back_to_chat() {
        // 未知模式与空串都不应让对话不可用
        assert_eq!(Mode::parse(""), Mode::Chat);
        assert_eq!(Mode::parse("bogus"), Mode::Chat);
    }

    #[test]
    fn default_is_chat() {
        assert_eq!(Mode::default(), Mode::Chat);
    }

    #[test]
    fn read_only_modes_allow_read_tools() {
        for m in [Mode::Chat, Mode::Plan] {
            for t in registry::names_by_kind(ToolKind::Read) {
                assert!(m.allows(t), "{} 应放行只读工具 {t}", m.as_str());
            }
        }
    }

    #[test]
    fn read_only_modes_block_every_mutating_tool() {
        for m in [Mode::Chat, Mode::Plan] {
            for t in registry::names_by_kind(ToolKind::Write)
                .into_iter()
                .chain(registry::names_by_kind(ToolKind::Exec))
            {
                assert!(
                    !m.allows(t),
                    "{} 必须拦截会改动工作区的工具 {t}",
                    m.as_str()
                );
            }
        }
    }

    /// 注册表里若真的一个写工具都没有，上面那条测试会空转通过，
    /// 所以再断言一次非空，防止工具改名后悄悄失效。
    #[test]
    fn mutating_tools_exist_to_be_blocked() {
        assert!(!registry::names_by_kind(ToolKind::Write).is_empty());
        assert!(!registry::names_by_kind(ToolKind::Exec).is_empty());
    }

    #[test]
    fn build_allows_everything() {
        for t in registry::names() {
            assert!(Mode::Build.allows(t), "执行模式应放行 {t}");
        }
    }

    #[test]
    fn only_build_is_unrestricted() {
        assert!(Mode::Build.is_unrestricted());
        assert!(!Mode::Chat.is_unrestricted());
        assert!(!Mode::Plan.is_unrestricted());
    }

    /// 只读两档的权限必须完全一致：Plan 相对 Chat 的差异是「是否产出清单」，
    /// 不是「能用什么工具」。若哪天两者的 kinds 分叉了，这条会拦住。
    #[test]
    fn chat_and_plan_share_identical_permissions() {
        assert_eq!(Mode::Chat.kinds(), Mode::Plan.kinds());
    }

    #[test]
    fn allows_defers_on_unknown_tool() {
        // 未注册工具交给调用方的 unknown tool 分支，不在模式层下结论
        assert!(Mode::Plan.allows("no_such_tool"));
    }

    #[test]
    fn reject_reason_lists_actually_allowed_tools() {
        for m in [Mode::Chat, Mode::Plan] {
            let msg = m.reject_reason("write_file");
            assert!(msg.contains("write_file"), "应指明被拒的工具");
            assert!(msg.contains("read_file"), "应列出真正可用的工具");
            // 可用清单里绝不能混入写/执行工具，否则模型会据此再次误调用
            for t in registry::names_by_kind(ToolKind::Write)
                .into_iter()
                .chain(registry::names_by_kind(ToolKind::Exec))
            {
                let listed = msg.split("可用工具为：").nth(1).unwrap_or_default();
                assert!(!listed.contains(t), "{t} 不应出现在 {} 的可用清单中", m.as_str());
            }
        }
    }

    #[test]
    fn reject_reason_stays_actionable() {
        // 必须指引用户切换模式，否则模型容易陷入反复重试。
        // 措辞用「执行模式」而非内部标识 Build。
        for m in [Mode::Chat, Mode::Plan] {
            let msg = m.reject_reason("run_script");
            assert!(msg.contains("执行模式"), "{} 的回执应指引切换", m.as_str());
        }
    }

    #[test]
    fn as_str_round_trips_through_parse() {
        for m in [Mode::Chat, Mode::Plan, Mode::Build] {
            assert_eq!(Mode::parse(m.as_str()), m);
        }
    }
}