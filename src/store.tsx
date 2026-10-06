import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { getWorkspace, removeSession, setWorkspace } from "./lib/api";

export type BlockKind = "text" | "action" | "notice" | "error";

export interface Block {
  kind: BlockKind;
  text: string;
}

export type ChatMessage =
  | { role: "user"; content: string }
  | { role: "assistant"; blocks: Block[] };

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
}

interface StoreValue extends State {
  activeSession: Session | undefined;
  switchWorkspace: (path: string) => Promise<void>;
  newSession: () => void;
  selectSession: (id: string) => void;
  deleteSession: (id: string) => Promise<void>;
}

const StoreContext = createContext<StoreValue | null>(null);

function createSession(workspace: string): Session {
  return {
    id: crypto.randomUUID(),
    title: "新对话",
    workspace,
    messages: [],
  };
}

export function StoreProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<State>({
    workspaces: [],
    workspace: "",
    sessions: [],
    activeId: "",
  });

  useEffect(() => {
    getWorkspace()
      .then((ws) => {
        setState((s) => {
          const workspaces = s.workspaces.includes(ws)
            ? s.workspaces
            : [ws, ...s.workspaces];
          let sessions = s.sessions;
          let activeId = s.activeId;
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

  const switchWorkspace = useCallback(async (path: string) => {
    await setWorkspace(path);
    setState((s) => {
      const workspaces = [path, ...s.workspaces.filter((w) => w !== path)];
      let sessions = s.sessions;
      let activeId = s.activeId;
      const active = sessions.find((x) => x.id === activeId);
      if (!active || active.workspace !== path) {
        const inWs = sessions.filter((x) => x.workspace === path);
        if (inWs.length > 0) {
          activeId = inWs[inWs.length - 1].id;
        } else {
          const ns = createSession(path);
          sessions = [...sessions, ns];
          activeId = ns.id;
        }
      }
      return { ...s, workspaces, workspace: path, sessions, activeId };
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
      switchWorkspace,
      newSession,
      selectSession,
      deleteSession,
    }),
    [state, switchWorkspace, newSession, selectSession, deleteSession],
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
