import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "../store";

export interface SavedSession {
  id: string;
  title: string;
  workspace: string;
  messages: ChatMessage[];
}

export interface ModelInfo {
  id: string;
  /** `{provider}:{id}` 复合 key（选中/切换主键） */
  key: string;
  provider: string;
  label: string;
  context_limit: number;
}

export function sendMessage(sessionId: string, message: string): Promise<void> {
  return invoke("send_message", { sessionId, message });
}

export function stopMessage(): Promise<void> {
  return invoke("stop_message");
}

export function removeSession(sessionId: string): Promise<void> {
  return invoke("remove_session", { sessionId });
}

export function loadSessions(): Promise<SavedSession[]> {
  return invoke("load_sessions");
}

export function saveSession(
  id: string,
  title: string,
  workspace: string,
  messages: ChatMessage[],
): Promise<void> {
  return invoke("save_session", { id, title, workspace, messages });
}

export function setWorkspace(path: string): Promise<string> {
  return invoke("set_workspace", { path });
}

export function getWorkspace(): Promise<string> {
  return invoke("get_workspace");
}

export function getGlobalRules(): Promise<string> {
  return invoke("get_global_rules");
}

export function setGlobalRules(rules: string): Promise<void> {
  return invoke("set_global_rules", { rules });
}

export function getModels(): Promise<ModelInfo[]> {
  return invoke("get_models");
}

export function getSelectedModel(): Promise<string> {
  return invoke("get_selected_model");
}

export function setSelectedModel(id: string): Promise<void> {
  return invoke("set_selected_model", { id });
}

export interface ApiConfig {
  provider: string;
  api_key: string;
}

export function getApiConfig(): Promise<ApiConfig> {
  return invoke("get_api_config");
}

export function setApiConfig(provider: string, apiKey: string): Promise<void> {
  return invoke("set_api_config", { provider, apiKey });
}

export function removeWorkspace(path: string, sessionIds: string[]): Promise<void> {
  return invoke("remove_workspace", { path, sessionIds });
}

export type ProviderKind =
  | "openrouter"
  | "openai"
  | "azure"
  | "ollama"
  | "llama_cpp"
  | "custom";

export interface ProviderConfig {
  id: string;
  kind: ProviderKind;
  name: string;
  base_url: string;
  api_key: string;
  headers: Record<string, unknown>;
  deployment: string | null;
  api_version: string | null;
  enabled: boolean;
}

export function getProviders(): Promise<ProviderConfig[]> {
  return invoke("get_providers");
}

export function setProviders(providers: ProviderConfig[]): Promise<void> {
  return invoke("set_providers", { providers });
}

export interface ProviderStatus {
  id: string;
  ok: boolean;
  latency_ms: number;
  message: string;
}

export function getProviderStatuses(): Promise<ProviderStatus[]> {
  return invoke("get_provider_statuses");
}

/** 拉取供应商远端模型并合并进配置，返回新增模型数。 */
export function refreshProviderModels(provider: string): Promise<number> {
  return invoke("refresh_provider_models", { provider });
}

/** Ollama 模型拉取（进度经 `ollama_pull` 事件推送），返回新增模型数。 */
export function pullOllamaModel(provider: string, model: string): Promise<number> {
  return invoke("pull_ollama_model", { provider, model });
}
