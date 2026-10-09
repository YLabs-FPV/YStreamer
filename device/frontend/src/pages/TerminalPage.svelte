<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import "@xterm/xterm/css/xterm.css";
  import { api, message } from "@/api/client";
  import Segmented from "@/components/ui/Segmented.svelte";
  import { btn } from "@/components/ui/buttons";

  type Phase = "loading" | "refused" | "connecting" | "open" | "closed";

  let host: HTMLDivElement;
  let phase = $state<Phase>("loading");
  let reason = $state("");
  let users = $state<string[]>([]);
  let user = $state("");

  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let socket: WebSocket | null = null;
  let observer: ResizeObserver | null = null;
  const encoder = new TextEncoder();

  async function load() {
    try {
      const info = await api.terminal();
      users = info.users;
      user = users[0] ?? "";
      if (info.refused) {
        reason = info.refused;
        phase = "refused";
      } else {
        connect();
      }
    } catch (e) {
      reason = message(e);
      phase = "refused";
    }
  }

  function connect() {
    socket?.close();
    if (!term || !fit) return;
    term.reset();
    fit.fit();
    phase = "connecting";

    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const query = new URLSearchParams({
      user,
      cols: String(term.cols),
      rows: String(term.rows),
    });
    const ws = new WebSocket(
      `${scheme}://${location.host}/api/terminal/shell?${query}`,
    );
    ws.binaryType = "arraybuffer";
    socket = ws;
    ws.onopen = () => {
      if (socket !== ws) return;
      phase = "open";
      term?.focus();
    };
    ws.onmessage = (e) => {
      if (socket === ws) term?.write(new Uint8Array(e.data as ArrayBuffer));
    };
    ws.onclose = () => {
      if (socket !== ws) return;
      socket = null;
      // Never opened: the device turned the request down
      if (phase === "connecting") load();
      else phase = "closed";
    };
  }

  function switchUser(next: string) {
    user = next;
    connect();
  }

  onMount(() => {
    term = new Terminal({
      cursorBlink: true,
      fontFamily:
        'ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace',
      fontSize: 13,
      scrollback: 5000,
      theme: { background: "#0b0e14", foreground: "#d8dee9" },
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    term.onData((data) => {
      if (socket?.readyState === WebSocket.OPEN)
        socket.send(encoder.encode(data));
    });
    term.onResize(({ cols, rows }) => {
      if (socket?.readyState === WebSocket.OPEN)
        socket.send(JSON.stringify({ cols, rows }));
    });
    observer = new ResizeObserver(() => fit?.fit());
    observer.observe(host);
    load();
  });

  onDestroy(() => {
    observer?.disconnect();
    const ws = socket;
    socket = null;
    ws?.close();
    term?.dispose();
  });
</script>

<div class="flex flex-col gap-3">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <p class="text-[12px] text-muted">
      {#if phase === "open"}
        A shell on this device, as <span class="font-mono">{user}</span>.
        Leaving the page closes it and stops what it was running.
      {:else if phase === "connecting" || phase === "loading"}
        Connecting…
      {:else if phase === "closed"}
        The shell has ended.
      {:else}
        {reason}
      {/if}
    </p>
    <div class="flex items-center gap-2">
      {#if users.length > 1 && phase !== "refused"}
        <div class="w-44">
          <Segmented
            options={users.map((u) => ({ label: u, value: u }))}
            value={user}
            onchange={switchUser}
          />
        </div>
      {/if}
      {#if phase === "closed"}
        <button type="button" class={btn.plain} onclick={connect}>
          New shell
        </button>
      {:else if phase === "refused"}
        <button type="button" class={btn.plain} onclick={load}>
          Try again
        </button>
      {/if}
    </div>
  </div>

  <div
    class="overflow-hidden rounded-xl border border-line bg-[#0b0e14] p-2 shadow-sm"
    class:opacity-50={phase !== "open"}
  >
    <div bind:this={host} class="h-[min(70vh,640px)]"></div>
  </div>
</div>
