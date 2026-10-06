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
  getWorkspace,
  loadSessions,
  removeSession,
  saveSession,
  sendMessage as sendMessageApi,
  setWorkspace,
  stopMessage as stopMessageApi,
} from "./lib/api";

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
}

interface StoreValue extends State {
  activeSession: Session | undefined;
  sendMessage: (text: string) => Promise<void>;
  stop: () => Promise<void>;
  switchWorkspace: (path: string) => Promise<void>;
  newSession: () => void;
  selectSession: (id: string) => void;
  deleteSession: (id: string) => Promise<void>;
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
  });

  const stateRef = useRef(state);
  stateRef.current = state;

  const bufferRef = useRef("");
  const streamRef = useRef<StreamTarget | null>(null);
  const sawErrorRef = useRef(false);

  const persistNow = useCallback(
    (id: string, title: string, workspace: string, messages: ChatMessage[]) => {
      saveSession(id, title, workspace, messages).catch(() => {});
    },
    [],
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
      const n = Math.min(
        chars.length,
        chars.length > 90 ? 6 : chars.length > 30 ? 3 : 1,
      );
      const take = chars.slice(0, n).join("");
      bufferRef.current = chars.slice(n).join("");
      appendBlocks(stream.sessionId, stream.messageId, (b) => mergeText(b, take));
    }, 30);
    return () => window.clearInterval(timer);
  }, [appendBlocks]);

  const drainBuffer = useCallback(async () => {
    const start = Date.now();
    while (bufferRef.current && Date.now() - start < 5000) {
      await new Promise((r) => setTimeout(r, 40));
    }
    const rest = bufferRef.current;
    bufferRef.current = "";
    const stream = streamRef.current;
    if (rest && stream) {
      appendBlocks(stream.sessionId, stream.messageId, (b) => mergeText(b, rest));
    }
  }, [appendBlocks]);

  useEffect(() => {
    Promise.all([getWorkspace(), loadSessions()])
      .then(([ws, saved]) => {
        setState((s) => {
          const restored: Session[] = saved.map((x) => ({
            id: x.id,
            title: x.title,
            workspace: x.workspace.replace(/\\/g, "/"),
            messages: x.messages,
          }));
          const workspaces = [ws, ...restored.map((r) => r.workspace), ...s.workspaces].filter(
            (w, i, arr) => arr.indexOf(w) === i,
          );
          let sessions = restored;
          let activeId = "";
          const active = restored.find((r) => r.workspace === ws) ?? restored[0];
          if (active) activeId = active.id;
          if (!sessions.some((x) => x.id === activeId)) {
            const ns = createSession(ws);
            sessions = [...sessions, ns];
            activeId = ns.id;
          }
          return { ...s, workspaces, workspace: ws, sessions, activeId };
        });
      })
      .catch(() => {});
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
        const inWs = sessions.filter((x) => x.workspace === canonical);
        if (inWs.length > 0) {
          activeId = inWs[inWs.length - 1].id;
        } else {
          const ns = createSession(canonical);
          sessions = [...sessions, ns];
          activeId = ns.id;
        }
      }
      return { ...s, workspaces, workspace: canonical, sessions, activeId };
    });
  }, []);

  const newSession = useCallback(() => {
    setState((s) => {
      const ns = createSession(s.workspace);
      return {
        ...s,
        sessions: [...s.sessions, ns],
        activeId: ns.id,
      };
    });
  }, []);

  const selectSession = useCallback((id: string) => {
    setState((s) => ({ ...s, activeId: id }));
  }, []);

  const deleteSession = useCallback(async (id: string) => {
    await removeSession(id);
    setState((s) => {
      let sessions = s.sessions.filter((x) => x.id !== id);
      let activeId = s.activeId;
      if (activeId === id) {
        const inWs = sessions.filter((x) => x.workspace === s.workspace);
        if (inWs.length > 0) {
          activeId = inWs[inWs.length - 1].id;
        } else {
          const ns = createSession(s.workspace);
          sessions = [...sessions, ns];
          activeId = ns.id;
        }
      }
      return { ...s, sessions, activeId };
    });
  }, []);

  const value = useMemo<StoreValue>(
    () => ({
      ...state,
      activeSession: state.sessions.find((x) => x.id === state.activeId),
      sendMessage,
      stop,
      switchWorkspace,
      newSession,
      selectSession,
      deleteSession,
    }),
    [state, sendMessage, stop, switchWorkspace, newSession, selectSession, deleteSession],
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
