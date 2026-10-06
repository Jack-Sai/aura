import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "../store";

export interface SavedSession {
  id: string;
  title: string;
  workspace: string;
  messages: ChatMessage[];
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
