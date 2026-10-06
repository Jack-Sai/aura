import { invoke } from "@tauri-apps/api/core";

export function sendMessage(sessionId: string, message: string): Promise<void> {
  return invoke("send_message", { sessionId, message });
}

export function removeSession(sessionId: string): Promise<void> {
  return invoke("remove_session", { sessionId });
}

export function setWorkspace(path: string): Promise<string> {
  return invoke("set_workspace", { path });
}

export function getWorkspace(): Promise<string> {
  return invoke("get_workspace");
}
