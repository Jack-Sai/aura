import type { ProviderKind } from "./api";

export interface ProviderPreset {
  /** 预设 id（写入 ProviderConfig.id 前缀，同时也是分组名） */
  id: string;
  /** 官方名称，用于分组标题回退 */
  label: string;
  kind: ProviderKind;
  /** 接口地址 */
  baseUrl: string;
  /**
   * 模型列表端点路径覆盖（相对 baseUrl）。
   * 仅当 chat 与 models 端点不同构、或列表端点不带 /v1 时需要。
   * - DeepSeek：官方列表端点为 GET /models，/v1 仅为 OpenAI SDK 兼容别名
   * - 阿里百炼：chat 走 /compatible-mode/v1，列表走 /api/v1/models
   */
  modelsPath?: string;
  /** 本地服务无需 API Key */
  local?: boolean;
}

/**
 * 内置厂商目录。地址均取自各家官方文档。
 *
 * 端点拼装由后端按「末段是否为版本号」自动推导，绝大多数厂商无需额外配置；
 * `modelsPath` 仅用于端点不同构的特殊厂商。
 */
export const PROVIDER_PRESETS: ProviderPreset[] = [
  // ── 国际 ──────────────────────────────────────────────
  {
    id: "openrouter",
    label: "OpenRouter",
    kind: "openrouter",
    baseUrl: "https://openrouter.ai/api/v1",
  },
  {
    id: "openai",
    label: "OpenAI",
    kind: "openai",
    baseUrl: "https://api.openai.com/v1",
  },
  {
    id: "deepseek",
    label: "DeepSeek 深度求索",
    kind: "openai",
    baseUrl: "https://api.deepseek.com",
    modelsPath: "/models",
  },
  {
    id: "kimi",
    label: "月之暗面 Kimi",
    kind: "openai",
    baseUrl: "https://api.moonshot.cn/v1",
  },
  {
    id: "groq",
    label: "Groq",
    kind: "openai",
    baseUrl: "https://api.groq.com/openai/v1",
  },
  {
    id: "xai",
    label: "xAI Grok",
    kind: "openai",
    baseUrl: "https://api.x.ai/v1",
  },
  {
    id: "together",
    label: "Together AI",
    kind: "openai",
    baseUrl: "https://api.together.xyz/v1",
  },

  // ── 国内 ──────────────────────────────────────────────
  {
    id: "zhipu",
    label: "智谱 GLM",
    kind: "openai",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
  },
  {
    id: "siliconflow",
    label: "硅基流动 SiliconFlow",
    kind: "openai",
    baseUrl: "https://api.siliconflow.cn/v1",
  },
  {
    id: "yi",
    label: "零一万物 Yi",
    kind: "openai",
    baseUrl: "https://api.lingyiwanwu.com/v1",
  },
  {
    id: "baichuan",
    label: "百川智能 Baichuan",
    kind: "openai",
    baseUrl: "https://api.baichuan-ai.com/v1",
  },
  {
    id: "stepfun",
    label: "阶跃星辰 Step",
    kind: "openai",
    baseUrl: "https://api.stepfun.com/v1",
  },
  {
    id: "minimax",
    label: "MiniMax 稀宇",
    kind: "openai",
    baseUrl: "https://api.minimax.io/v1",
  },
  {
    id: "dashscope",
    label: "阿里百炼 通义千问",
    kind: "openai",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    modelsPath: "/api/v1/models",
  },
  {
    id: "doubao",
    label: "火山方舟 豆包",
    kind: "openai",
    baseUrl: "https://ark.cn-beijing.volces.com/api/v3",
  },
  {
    id: "hunyuan",
    label: "腾讯混元",
    kind: "openai",
    baseUrl: "https://api.hunyuan.cloud.tencent.com/v1",
  },

  // ── 本地 ──────────────────────────────────────────────
  {
    id: "ollama",
    label: "Ollama",
    kind: "ollama",
    baseUrl: "http://127.0.0.1:11434",
    local: true,
  },
  {
    id: "llamacpp",
    label: "llama.cpp",
    kind: "llama_cpp",
    baseUrl: "http://127.0.0.1:8080",
    local: true,
  },
];

export const CUSTOM_PRESET_ID = "custom";

/** 「自定义」厂商：地址完全由用户填写，无预设默认地址 */
export const CUSTOM_PRESET: ProviderPreset = {
  id: CUSTOM_PRESET_ID,
  label: "自定义",
  kind: "custom",
  baseUrl: "",
};

/** 全部可选项：内置厂商 + 自定义 + Azure（Azure 地址由用户填资源域名） */
export const ALL_PRESETS: ProviderPreset[] = [
  ...PROVIDER_PRESETS,
  {
    id: "azure",
    label: "Azure OpenAI",
    kind: "azure",
    baseUrl: "https://your-resource.openai.azure.com",
  },
  CUSTOM_PRESET,
];

export function presetById(id: string): ProviderPreset | undefined {
  return ALL_PRESETS.find((p) => p.id === id);
}

/**
 * 分组标题：用户自定义名优先，回退到预设官方名，最后回退到 provider id。
 * 用户可以把「kimi」改名成「公司 Kimi」，模型列表的分组标题随之更新。
 */
export function providerDisplayName(
  providerId: string,
  customName: string | undefined,
): string {
  const name = customName?.trim();
  if (name) return name;
  return presetById(providerId)?.label ?? providerId;
}