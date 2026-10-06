import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-6 p-8">
      <h1 className="text-4xl font-semibold tracking-[-0.03em]">Aura</h1>
      <p className="text-subtle">Minimal surface. Maximum logic.</p>

      <form
        className="flex w-full max-w-sm gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          greet();
        }}
      >
        <input
          id="greet-input"
          className="min-w-0 flex-1 rounded-card border border-line bg-surface px-4 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-brand"
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <button
          type="submit"
          className="rounded-full bg-brand px-5 py-2 text-sm font-medium text-white transition-colors hover:bg-brand-strong"
        >
          Greet
        </button>
      </form>

      <p className="font-mono text-sm text-subtle">{greetMsg}</p>
    </main>
  );
}

export default App;
