import { useStore, type Block } from "../store";
import Markdown from "./Markdown";

function UserBubble({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[75%] whitespace-pre-wrap break-words rounded-card bg-bubble px-4 py-2.5 text-[15px] leading-relaxed">
        {content}
      </div>
    </div>
  );
}

function AssistantMessage({ blocks }: { blocks: Block[] }) {
  return (
    <div className="flex flex-col gap-4">
      {blocks
        .filter((b) => b.kind === "text")
        .map((b, i) => (
          <Markdown key={i} content={b.text} />
        ))}
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex min-h-full items-center justify-center">
      <div className="text-center">
        <h2 className="text-2xl font-semibold tracking-[-0.03em]">Aura</h2>
        <p className="mt-2 text-sm text-subtle">
          Minimal surface. Maximum logic.
        </p>
      </div>
    </div>
  );
}

export default function MessageList() {
  const { activeSession } = useStore();
  const messages = activeSession?.messages ?? [];

  if (messages.length === 0) {
    return <EmptyState />;
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-6 py-6">
      {messages.map((m, i) =>
        m.role === "user" ? (
          <UserBubble key={i} content={m.content} />
        ) : (
          <AssistantMessage key={i} blocks={m.blocks} />
        ),
      )}
    </div>
  );
}
