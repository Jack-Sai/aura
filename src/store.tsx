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
  saveTodos as saveTodosApi,
  sendMessage as sendMessageApi,
  setSelectedModel as setSelectedModelApi,
  setWorkspace,
  stopMessage as stopMessageApi,
  type ModelInfo,
} from "./lib/api";
import { type Mode } from "./lib/mode";
import {
  applyAutoClear,
  isTodoStatus,
  normalizeTodos,
  nextTodoId,
  type Todo,
  type TodoStatus,
} from "./lib/todo";

// 统一工作区键：反斜杠转 /，并剥离 Windows 扩展路径的 //?/ 前缀，
// 保证与 Rust 侧 canonicalize 后的键一致
const normWs = (p: string) => p.replace(/\\/g, "/").replace(/^\/\/\?\//, "");

export type BlockKind = "text" | "action" | "notice" | "error";

export interface Block {
  kind: BlockKind;
  text: string;
  /** action 块专用：工具是否执行成功 */
  ok?: boolean;
  /** action 块专用：可折叠的详细输出（命令 stdout/stderr、失败原因） */
  detail?: string;
}

export type ChatMessage =
  | { id: string; role: "user"; content: string }
  | { id: string; role: "assistant"; blocks: Block[] };

export interface Session {
  id: string;
  title: string;
  workspace: string;
  messages: ChatMessage[];
  /** 任务清单。旧会话可能没有，按空处理 */
  todos: Todo[];
}

interface State {
  workspaces: string[];
  workspace: string;
  sessions: Session[];
  activeId: string;
  busy: boolean;
  models: ModelInfo[];
  selectedModel: string;
  /** 当前会话模式，每次发送时随消息带上 */
  mode: Mode;
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
  setMode: (m: Mode) => void;
  deleteTodoStep: (id: string) => void;
  addTodoStep: (title: string) => void;
  setTodoStatus: (id: string, status: TodoStatus) => void;
  refreshModels: () => Promise<void>;
}

const StoreContext = createContext<StoreValue | null>(null);

interface StreamTarget {
  sessionId: string;
  messageId: string;
  /**
   * 打字机正在写入的 text block 下标。
   *
   * 不能只看「最后一个 block」：模型输出「先说一句」再调工具时，
   * 动作卡片会在文字**还没被打字机消费**时就插入。若打字机届时才发现
   * 末块不是 text，就会新开一块——文字被挤到动作卡片**后面**，
   * 与模型实际输出顺序相反。
   */
  textIndex: number;
}

function createSession(workspace: string): Session {
  return {
    id: crypto.randomUUID(),
    title: "新对话",
    workspace,
    messages: [],
    todos: [],
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

/**
 * 把一段文本合并到「打字机当前正在写」的 block 上；该位置还不是 text
 * （尚未写出任何字符，或已被别的 block 占据）就在末尾新建。
 *
 * 返回新的下标：调用方要把它写回 [`StreamTarget.textIndex`]，否则后续
 * 打字机又会去合并到旧位置，文本会跳到 action 卡片前面。
 */
export function appendText(
  blocks: Block[],
  index: number,
  text: string,
): { blocks: Block[]; index: number } {
  const i = Math.min(index, blocks.length);
  const cur = blocks[i];
  if (cur && cur.kind === "text") {
    const next = [...blocks];
    next[i] = { ...cur, text: cur.text + text };
    return { blocks: next, index: i };
  }
  return { blocks: [...blocks, { kind: "text", text }], index: blocks.length };
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
    mode: "chat",
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

  /**
   * 把缓冲区里尚未渲染的文字**立即**落到消息里。
   *
   * action / notice / error 插入之前必须先调它：这些块走的是直接 append，
   * 不经过打字机缓冲。若不排空，模型「先说一句再调工具」时动作卡片会
   * 排在那句话**前面**。
   */
  const flushBuffer = useCallback(() => {
    const buf = bufferRef.current;
    const stream = streamRef.current;
    if (!buf || !stream) return;
    bufferRef.current = "";
    appendBlocks(stream.sessionId, stream.messageId, (b) => {
      const r = appendText(b, stream.textIndex, buf);
      stream.textIndex = r.index;
      return r.blocks;
    });
  }, [appendBlocks]);

  /** 打字机消费一段文字：合并到当前正在写的 text block。 */
  const writeToStream = useCallback(
    (stream: StreamTarget, text: string) => {
      appendBlocks(stream.sessionId, stream.messageId, (b) => {
        const r = appendText(b, stream.textIndex, text);
        stream.textIndex = r.index;
        return r.blocks;
      });
    },
    [appendBlocks],
  );

  /**
   * 在消息末尾插入非文本块（动作 / 提示 / 错误）。
   *
   * 插入前先排空缓冲区，否则模型「先说一句再调工具」时动作卡片会排到
   * 那句话**前面**。插入后还要把 `textIndex` 推到新块之后——后续正文必须
   * 写在它后面，否则打字机会把新文字合并到动作卡片前面。
   */
  const insertTailBlock = useCallback(
    (stream: StreamTarget, block: Block) => {
      flushBuffer();
      appendBlocks(stream.sessionId, stream.messageId, (b) => {
        // 新块落在 b.length，下一个 text 块的位置就是 b.length + 1
        stream.textIndex = b.length + 1;
        return [...b, block];
      });
    },
    [appendBlocks, flushBuffer],
  );

  useEffect(() => {
    let disposed = false;
    const unsubs: Array<() => void> = [];

    // 泛型化而非写死 payload：新事件（如 todo_plan）携带的是 todos 而非
    // text/ok/detail，写死类型就只能改签名或塞进 any
    const on = async <T,>(name: string, cb: (payload: T) => void) => {
      const un = await listen<T>(name, (e) => cb(e.payload));
      if (disposed) un();
      else unsubs.push(un);
    };

    on<{ session_id: string; text?: string }>("agent:chunk", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      bufferRef.current += p.text ?? "";
    });
    on<{ session_id: string; text?: string; ok?: boolean; detail?: string }>(
      "agent:action",
      (p) => {
        const stream = streamRef.current;
        if (!stream || stream.sessionId !== p.session_id) return;
        insertTailBlock(stream, {
          // ok 由后端给出；旧版本事件无此字段时按成功处理
          kind: "action",
          text: p.text ?? "",
          ok: p.ok !== false,
          detail: p.detail || undefined,
        });
      },
    );
    on<{ session_id: string; text?: string }>("agent:notice", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      insertTailBlock(stream, { kind: "notice", text: p.text ?? "" });
    });
    on<{ session_id: string; text?: string }>("agent:error", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      sawErrorRef.current = true;
      insertTailBlock(stream, { kind: "error", text: p.text ?? "" });
    });
    on<{ session_id: string }>("agent:done", (p) => {
      const stream = streamRef.current;
      if (!stream || stream.sessionId !== p.session_id) return;
      // 收尾时可能仍有未消费的正文，先落位再判断是否空消息
      flushBuffer();
      appendBlocks(stream.sessionId, stream.messageId, (b) =>
        b.length === 0 ? [{ kind: "text", text: "" }] : b,
      );
    });

    // 规划模式产出的任务清单。注意这里**不要求** streamRef 匹配：
    // 清单属于会话级状态，即使此刻正在流式输出也应立即反映到面板
    on<{ session_id: string; todos: unknown }>("agent:todo_plan", (p) => {
      setState((s) => ({
        ...s,
        sessions: s.sessions.map((sess) =>
          sess.id === p.session_id
            ? { ...sess, todos: normalizeTodos(p.todos) }
            : sess,
        ),
      }));
    });

    // 单步状态推进。同样不要求 streamRef 匹配，且**不落盘**——
    // 后端已把最终状态写进 todos_json，这里只更新内存用于即时显示，
    // 避免每次勾选都触发一次写放大
    on<{ session_id: string; id: string; status: string }>(
      "agent:todo_update",
      (p) => {
        if (!isTodoStatus(p.status)) return;
        setState((s) => ({
          ...s,
          sessions: s.sessions.map((sess) => {
            if (sess.id !== p.session_id) return sess;
            const next = sess.todos.map((t) =>
              t.id === p.id ? { ...t, status: p.status as TodoStatus } : t,
            );
            // 全部完成后自动清空，任务做完即清场
            return { ...sess, todos: applyAutoClear(next) };
          }),
        }));
      },
    );

    return () => {
      disposed = true;
      unsubs.forEach((u) => u());
    };
  }, [insertTailBlock, flushBuffer]);

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
      writeToStream(stream, take);
    }, 30);
    return () => window.clearInterval(timer);
  }, [writeToStream]);

  const drainBuffer = useCallback(async () => {
    const start = Date.now();
    while (bufferRef.current && Date.now() - start < 5000) {
      const stream = streamRef.current;
      if (!stream) break;
      const chars = Array.from(bufferRef.current);
      const n = Math.max(1, Math.ceil(chars.length / 3));
      const take = chars.slice(0, n).join("");
      bufferRef.current = chars.slice(n).join("");
      writeToStream(stream, take);
      await new Promise((r) => setTimeout(r, 30));
    }
    const rest = bufferRef.current;
    bufferRef.current = "";
    const stream = streamRef.current;
    if (rest && stream) {
      writeToStream(stream, rest);
    }
  }, [writeToStream]);

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
              // 旧版本会话没有 todos 字段，按空清单处理
              todos: normalizeTodos(x.todos),
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

  const setMode = useCallback((m: Mode) => {
    setState((s) => (s.mode === m ? s : { ...s, mode: m }));
  }, []);

  /**
   * 以「乐观更新 + 后端回填」的方式保存清单。
   *
   * 先改本地 state 让面板立刻响应，再落盘；后端会按与模型产出
   * 相同的规则清洗（去空标题、去重复 id），返回清洗结果后再对齐一次，
   * 避免前端与落盘态出现分歧。
   */
  const commitTodos = useCallback(
    async (sessionId: string, todos: Todo[]) => {
      // 自动清空放在这里而不是各调用点：这是清单状态变更的唯一出口，
      // 收在一处才不会漏掉某条路径（模型事件、勾选、新增都要经过）
      const clean = applyAutoClear(normalizeTodos(todos));
      setState((s) => ({
        ...s,
        sessions: s.sessions.map((sess) =>
          sess.id === sessionId ? { ...sess, todos: clean } : sess,
        ),
      }));
      try {
        const saved = await saveTodosApi(sessionId, clean);
        setState((s) => ({
          ...s,
          sessions: s.sessions.map((sess) =>
            sess.id === sessionId
              ? { ...sess, todos: normalizeTodos(saved) }
              : sess,
          ),
        }));
      } catch {
        /* 落盘失败保留内存态，下次操作会重试 */
      }
    },
    [],
  );

  const deleteTodoStep = useCallback(
    (id: string) => {
      const sess = stateRef.current.sessions.find(
        (x) => x.id === stateRef.current.activeId,
      );
      if (!sess) return;
      void commitTodos(sess.id, sess.todos.filter((t) => t.id !== id));
    },
    [commitTodos],
  );

  const addTodoStep = useCallback(
    (title: string) => {
      const sess = stateRef.current.sessions.find(
        (x) => x.id === stateRef.current.activeId,
      );
      const text = title.trim();
      if (!sess || !text) return;
      void commitTodos(sess.id, [
        ...sess.todos,
        { id: nextTodoId(sess.todos), title: text, status: "pending" },
      ]);
    },
    [commitTodos],
  );

  const setTodoStatus = useCallback(
    (id: string, status: TodoStatus) => {
      const sess = stateRef.current.sessions.find(
        (x) => x.id === stateRef.current.activeId,
      );
      if (!sess) return;
      void commitTodos(
        sess.id,
        sess.todos.map((t) => (t.id === id ? { ...t, status } : t)),
      );
    },
    [commitTodos],
  );

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
      streamRef.current = { sessionId, messageId: assistantMsg.id, textIndex: 0 };
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
        await sendMessageApi(sessionId, text, stateRef.current.mode);
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
      // 传当前会话：后端据此校验归属，避免停错流
      await stopMessageApi(stateRef.current.activeId);
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
      setMode,
      deleteTodoStep,
      addTodoStep,
      setTodoStatus,
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
      setMode,
      deleteTodoStep,
      addTodoStep,
      setTodoStatus,
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
