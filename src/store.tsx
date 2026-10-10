import { listen } from "@tauri-apps/api/event";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  getModels,
  getSelectedModel,
  getWorkspace,
  loadSessions,
  removeSession,
  removeWorkspace as removeWorkspaceApi,
  saveSession,
  sendMessage as sendMessageApi,
  setSelectedModel as setSelectedModelApi,
  setWorkspace,
  stopMessage as stopMessageApi,
  type ModelInfo,
} from "./lib/api";

// 统一工作区键：反斜杠转 /，并剥离 Windows 扩展路径的 //?/ 前缀，
// 保证与 Rust 侧 canonicalize 后的键一致
const normWs = (p: string) => p.replace(/\\/g, "/").replace(/^\/\/\?\//, "");

export type BlockKind = "text" | "action" | "notice" | "error";

export interface Block {
  kind: BlockKind;
  text: string;
}

export type ChatMessage =
  | { id: string; role: "user"; content: string }
  | { id: string; role: "assistant"; blocks: Block[] };

export interface Session {
  id: string;
  title: string;
  workspace: string;
  messages: ChatMessage[];
}

interface State {
  workspaces: string[];
  workspace: string;
  sessions: Session[];
  activeId: string;
  busy: boolean;
  models: ModelInfo[];
  selectedModel: string;
}

interface StoreValue extends State {
  activeSession: Session | undefined;
  renamingId: string | null;
  startRename: (id: string) => void;
  cancelRename: () => void;
  renameSession: (id: string, title: string) => void;
  sendMessage: (text: string) => Promise<void>;
  stop: () => Promise<void>;
  switchWorkspace: (path: string) => Promise<void>;
  newSession: (workspace?: string) => Promise<void>;
  selectSession: (id: string) => void;
  openSession: (id: string) => Promise<void>;
  deleteSession: (id: string) => void;
  removeWorkspace: (path: string) => Promise<void>;
  setModel: (id: string) => Promise<void>;
  refreshModels: () => Promise<void>;
}

const StoreContext = createContext<StoreValue | null>(null);

interface StreamTarget {
  sessionId: string;
  messageId: string;
}

function createSession(workspace: string): Session {
  return {
    id: crypto.randomUUID(),
    title: "新对话",
    workspace,
    messages: [],
  };
}

function deriveTitle(text: string): string {
  const t = text.trim();
  return t.length > 20 ? `${t.slice(0, 20)}…` : t;
}

/**
 * 选定某工作区应激活的会话：优先未发送消息的空对话（与「新建对话」语义一致），
 * 否则取最近更新的一个。sessions 从数据库加载时按 updated_at DESC 排列，故首项最新。
 */
function pickSessionInWorkspace(
  sessions: Session[],
  ws: string,
): Session | undefined {
  const inWs = sessions.filter((x) => x.workspace === ws);
  return inWs.find((x) => x.messages.length === 0) ?? inWs[0];
}

function mergeText(blocks: Block[], text: string): Block[] {
  const last = blocks[blocks.length - 1];
  if (last && last.kind === "text") {
    return [...blocks.slice(0, -1), { kind: "text", text: last.text + text }];
  }
  return [...blocks, { kind: "text", text }];
}

export function StoreProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<State>({
    workspaces: [],
    workspace: "",
    sessions: [],
    activeId: "",
    busy: false,
    models: [],
    selectedModel: "",
  });

  const stateRef = useRef(state);
  stateRef.current = state;

  const [renamingId, setRenamingId] = useState<string | null>(null);

  const startRename = useCallback((id: string) => setRenamingId(id), []);
  const cancelRename = useCallback(() => setRenamingId(null), []);

  const bufferRef = useRef("");
  const streamRef = useRef<StreamTarget | null>(null);
  const sawErrorRef = useRef(false);

  const persistNow = useCallback(
    (id: string, title: string, workspace: string, messages: ChatMessage[]) => {
      saveSession(id, title, workspace, messages).catch(() => {});
    },
    [],
  );

  const renameSession = useCallback(
    (id: string, title: string) => {
      const t = title.trim();
      if (!t) return;
      const sess = stateRef.current.sessions.find((x) => x.id === id);
      if (!sess) return;
      setState((s) => ({
        ...s,
        sessions: s.sessions.map((x) => (x.id === id ? { ...x, title: t } : x)),
      }));
      persistNow(id, t, sess.workspace, sess.messages);
    },
    [persistNow],
  );

  const appendBlocks = useCallback(
    (sessionId: string, messageId: string, updater: (blocks: Block[]) => Block[]) => {
      setState((s) => ({
        ...s,
        sessions: s.sessions.map((sess) => {
          if (sess.id !== sessionId) return sess;
          return {
            ...sess,
            messages: sess.messages.map((m) =>
              m.id === messageId && m.role === "assistant"
                ? { ...m, blocks: updater(m.blocks) }
                : m,
            ),
          };
        }),
      }));
    },
    [],
  );

  useEffect(() => {
    let disposed = false;
    const unsubs: Array<() => void> = [];

    const on = async (
      name: string,
      cb: (payload: { session_id: string; text?: string }) => void,
    ) => {
      const un = await listen<{ session_id: string; text?: string }>(name, (e) =>
        cb(e.payload),
      );
      if (disposed) un();
      else unsubs.push(un);
    };

    on("agent:chunk", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      bufferRef.current += p.text ?? "";
    });
    on("agent:action", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      appendBlocks(stream.sessionId, stream.messageId, (b) => [
        ...b,
        { kind: "action", text: p.text ?? "" },
      ]);
    });
    on("agent:notice", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      appendBlocks(stream.sessionId, stream.messageId, (b) => [
        ...b,
        { kind: "notice", text: p.text ?? "" },
      ]);
    });
    on("agent:error", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      sawErrorRef.current = true;
      appendBlocks(stream.sessionId, stream.messageId, (b) => [
        ...b,
        { kind: "error", text: p.text ?? "" },
      ]);
    });
    on("agent:done", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      appendBlocks(stream.sessionId, stream.messageId, (b) =>
        b.length === 0 ? [{ kind: "text", text: "" }] : b,
      );
    });

    return () => {
      disposed = true;
      unsubs.forEach((u) => u());
    };
  }, [appendBlocks]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const buf = bufferRef.current;
      const stream = streamRef.current;
      if (!buf || !stream) return;
      const chars = Array.from(buf);
      const backlog = chars.length;
      // 自适应消费：正常流逐字推进（打字机节奏），
      // 网络突发积压时按时间预算加速，约 0.5s 内追平，避免卡顿滞后
      let n: number;
      if (backlog <= 60) {
        n = backlog > 40 ? 2 : 1;
      } else if (backlog <= 300) {
        n = Math.ceil(backlog / 30);
      } else {
        n = Math.ceil(backlog / 15) + 4;
      }
      n = Math.min(Math.max(n, 1), backlog);
      const take = chars.slice(0, n).join("");
      bufferRef.current = chars.slice(n).join("");
      appendBlocks(stream.sessionId, stream.messageId, (b) => mergeText(b, take));
    }, 30);
    return () => window.clearInterval(timer);
  }, [appendBlocks]);

  const drainBuffer = useCallback(async () => {
    const start = Date.now();
    while (bufferRef.current && Date.now() - start < 5000) {
      const stream = streamRef.current;
      if (!stream) break;
      const chars = Array.from(bufferRef.current);
      const n = Math.max(1, Math.ceil(chars.length / 3));
      const take = chars.slice(0, n).join("");
      bufferRef.current = chars.slice(n).join("");
      appendBlocks(stream.sessionId, stream.messageId, (b) => mergeText(b, take));
      await new Promise((r) => setTimeout(r, 30));
    }
    const rest = bufferRef.current;
    bufferRef.current = "";
    const stream = streamRef.current;
    if (rest && stream) {
      appendBlocks(stream.sessionId, stream.messageId, (b) => mergeText(b, rest));
    }
  }, [appendBlocks]);

  useEffect(() => {
    Promise.all([getWorkspace(), loadSessions(), getModels(), getSelectedModel()])
      .then(([rawWs, saved, models, selectedModel]) => {
        const ws = normWs(rawWs);
        setState((s) => {
          // 过滤历史脏行（工作区为空），避免侧边栏出现空白工作区分组；
          // 每工作区最多保留一个空对话，多余的丢弃
          const blankSeen = new Set<string>();
          const restored: Session[] = saved
            .map((x) => ({
              id: x.id,
              title: x.title,
              workspace: normWs(x.workspace),
              messages: x.messages,
            }))
            .filter((r) => {
              if (!r.workspace) return false;
              if (r.messages.length > 0) return true;
              if (blankSeen.has(r.workspace)) return false;
              blankSeen.add(r.workspace);
              return true;
            });
          const workspaces = [ws, ...restored.map((r) => r.workspace), ...s.workspaces]
            .map(normWs)
            .filter((w, i, arr) => arr.indexOf(w) === i);
          let sessions = restored;
          let activeId = "";
          const picked =
            pickSessionInWorkspace(restored, ws) ?? restored[0];
          if (picked) activeId = picked.id;
          if (!sessions.some((x) => x.id === activeId)) {
            const ns = createSession(ws);
            sessions = [...sessions, ns];
            activeId = ns.id;
          }
          return {
            ...s,
            workspaces,
            workspace: ws,
            sessions,
            activeId,
            models,
            selectedModel,
          };
        });
      })
      .catch(() => {});
  }, []);

  const setModel = useCallback(async (id: string) => {
    try {
      await setSelectedModelApi(id);
      setState((s) => ({ ...s, selectedModel: id }));
    } catch {
      /* 无效模型保持原选择 */
    }
  }, []);

  const refreshModels = useCallback(async () => {
    try {
      const [models, selectedModel] = await Promise.all([
        getModels(),
        getSelectedModel(),
      ]);
      setState((s) => ({ ...s, models, selectedModel }));
    } catch {
      /* 刷新失败保持现有列表 */
    }
  }, []);

  const sendMessage = useCallback(
    async (text: string) => {
      if (state.busy) return;
      const sessionId = state.activeId;
      if (!sessionId) return;
      setState((s) => ({ ...s, busy: true }));
      sawErrorRef.current = false;
      bufferRef.current = "";
      const userMsg: ChatMessage = {
        id: crypto.randomUUID(),
        role: "user",
        content: text,
      };
      const assistantMsg: ChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        blocks: [],
      };
      const current = stateRef.current.sessions.find((x) => x.id === sessionId);
      if (!current) {
        setState((s) => ({ ...s, busy: false }));
        return;
      }
      const title =
        current.title === "新对话" ? deriveTitle(text) : current.title;
      const messages = [...current.messages, userMsg, assistantMsg];
      streamRef.current = { sessionId, messageId: assistantMsg.id };
      setState((s) => ({
        ...s,
        sessions: s.sessions.map((sess) =>
          sess.id !== sessionId
            ? sess
            : { ...sess, title, messages },
        ),
      }));
      persistNow(sessionId, title, current.workspace, messages);
      try {
        await sendMessageApi(sessionId, text);
      } catch (e) {
        if (!sawErrorRef.current) {
          appendBlocks(sessionId, assistantMsg.id, (b) => [
            ...b,
            { kind: "error", text: String(e) },
          ]);
        }
      } finally {
        await drainBuffer();
        streamRef.current = null;
        setState((s) => ({ ...s, busy: false }));
        await new Promise((r) => setTimeout(r, 0));
        const latest = stateRef.current.sessions.find(
          (x) => x.id === sessionId,
        );
        if (latest) {
          persistNow(sessionId, latest.title, latest.workspace, latest.messages);
        }
      }
    },
    [state.busy, state.activeId, appendBlocks, drainBuffer, persistNow],
  );

  const stop = useCallback(async () => {
    try {
      await stopMessageApi();
    } catch {
      /* 停止命令失败时维持现状 */
    }
  }, []);

  const switchWorkspace = useCallback(async (path: string) => {
    const canonical = await setWorkspace(path);
    setState((s) => {
      const workspaces = [canonical, ...s.workspaces.filter((w) => w !== canonical)].filter(
        (w, i, arr) => arr.indexOf(w) === i,
      );
      let sessions = s.sessions;
      let activeId = s.activeId;
      const active = sessions.find((x) => x.id === activeId);
      if (!active || active.workspace !== canonical) {
        const picked = pickSessionInWorkspace(sessions, canonical);
        if (picked) {
          activeId = picked.id;
        } else {
          const ns = createSession(canonical);
          sessions = [...sessions, ns];
          activeId = ns.id;
        }
      }
      return { ...s, workspaces, workspace: canonical, sessions, activeId };
    });
  }, []);

  const newSession = useCallback(async (workspace?: string) => {
    const ws = workspace ?? stateRef.current.workspace;
    if (ws !== stateRef.current.workspace) {
      try {
        await setWorkspace(ws);
      } catch {
        return;
      }
    }
    setState((s) => {
      // 每个工作区只保留一个空对话：已有空对话则直接聚焦，不重复创建，
      // 避免工作区被「新对话」挤满
      const blank = s.sessions.find(
        (x) => x.workspace === ws && x.messages.length === 0,
      );
      if (blank) {
        return { ...s, workspace: ws, activeId: blank.id };
      }
      const ns = createSession(ws);
      return {
        ...s,
        workspace: ws,
        sessions: [...s.sessions, ns],
        activeId: ns.id,
      };
    });
  }, []);

  const selectSession = useCallback((id: string) => {
    setState((s) => ({ ...s, activeId: id }));
  }, []);

  const openSession = useCallback(async (id: string) => {
    const sess = stateRef.current.sessions.find((x) => x.id === id);
    if (!sess) return;
    if (sess.workspace !== stateRef.current.workspace) {
      try {
        await setWorkspace(sess.workspace);
      } catch {
        return;
      }
    }
    setState((s) => ({ ...s, workspace: sess.workspace, activeId: id }));
  }, []);

  const deleteSession = useCallback((id: string) => {
    removeSession(id).catch(() => {});
    setState((s) => {
      let sessions = s.sessions.filter((x) => x.id !== id);
      let activeId = s.activeId;
      if (activeId === id) {
        const picked = pickSessionInWorkspace(sessions, s.workspace);
        if (picked) {
          activeId = picked.id;
        } else {
          const ns = createSession(s.workspace);
          sessions = [...sessions, ns];
          activeId = ns.id;
        }
      }
      return { ...s, sessions, activeId };
    });
  }, []);

  const removeWorkspace = useCallback(async (path: string) => {
    const s0 = stateRef.current;
    if (path === s0.workspace) return;
    const ids = s0.sessions.filter((x) => x.workspace === path).map((x) => x.id);
    removeWorkspaceApi(path, ids).catch(() => {});
    setState((s) => ({
      ...s,
      workspaces: s.workspaces.filter((w) => w !== path),
      sessions: s.sessions.filter((x) => x.workspace !== path),
    }));
  }, []);

  const value = useMemo<StoreValue>(
    () => ({
      ...state,
      activeSession: state.sessions.find((x) => x.id === state.activeId),
      renamingId,
      startRename,
      cancelRename,
      renameSession,
      sendMessage,
      stop,
      switchWorkspace,
      newSession,
      selectSession,
      openSession,
      deleteSession,
      removeWorkspace,
      setModel,
      refreshModels,
    }),
    [
      state,
      renamingId,
      startRename,
      cancelRename,
      renameSession,
      sendMessage,
      stop,
      switchWorkspace,
      newSession,
      selectSession,
      openSession,
      deleteSession,
      removeWorkspace,
      setModel,
      refreshModels,
    ],
  );

  return (
    <StoreContext.Provider value={value}>{children}</StoreContext.Provider>
  );
}

export function useStore(): StoreValue {
  const value = useContext(StoreContext);
  if (!value) {
    throw new Error("useStore 必须在 StoreProvider 内使用");
  }
  return value;
}
