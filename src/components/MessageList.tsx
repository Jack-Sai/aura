import { useStore, type Block } from "../store";
import { ActionRow, ErrorRow, NoticeRow } from "./ActionRow";
import Markdown from "./Markdown";

function UserBubble({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[75%] whitespace-pre-wrap break-words rounded-card bg-bubble px-4 py-3 text-[15px] leading-relaxed">
        {content}
      </div>
    </div>
  );
}

function renderBlock(block: Block, i: number) {
  switch (block.kind) {
    case "text":
      return <Markdown key={i} content={block.text} />;
    case "action":
      return <ActionRow key={i} text={block.text} />;
    case "notice":
      return <NoticeRow key={i} text={block.text} />;
    case "error":
      return <ErrorRow key={i} text={block.text} />;
  }
}

function AssistantMessage({ blocks }: { blocks: Block[] }) {
  if (blocks.length === 0) {
    return (
      <div className="min-h-[1.6em] leading-relaxed">
        <span className="aura-breathing-cursor" aria-hidden="true" />
      </div>
    );
  }
  return <div className="flex flex-col gap-3">{blocks.map(renderBlock)}</div>;
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
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-6 py-6 xl:max-w-5xl 2xl:max-w-6xl">
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
