<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, message } from "@/api/client";
  import type { WireguardStatus } from "@/types/wireguard";
  import { bytes } from "@/stores/recording.svelte";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";

  // WireGuard renews its handshake every two minutes while there's traffic;
  // well past that, the other end has stopped answering
  const STALE_SECS = 180;

  let status = $state<WireguardStatus | null>(null);
  let busy = $state(false);
  let confirmRemove = $state(false);
  let input = $state<HTMLInputElement>();
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh() {
    if (busy) return;
    try {
      status = await api.wireguard();
    } catch {
      // Shown as offline elsewhere
    }
  }

  async function run(what: () => Promise<WireguardStatus>, done?: string) {
    busy = true;
    try {
      status = await what();
      if (done) snackbar.show(done);
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      busy = false;
    }
  }

  function picked(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    if (input) input.value = "";
    if (file)
      run(() => api.wireguardImport(file), "WireGuard configuration imported");
  }

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 5000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  function ago(secs: number) {
    if (secs < 60) return `${secs} s ago`;
    if (secs < 3600) return `${Math.floor(secs / 60)} min ago`;
    return `${Math.floor(secs / 3600)} h ago`;
  }

  const peer = $derived(status?.peers[0] ?? null);
  const answering = $derived(
    peer?.handshake_age_secs != null && peer.handshake_age_secs < STALE_SECS,
  );
  const everything = $derived(
    !!peer?.allowed_ips.some((a) => a === "0.0.0.0/0" || a === "::/0"),
  );

  const summary = $derived.by(() => {
    if (status?.state === "off")
      return { text: "Imported, but switched off", tone: "text-muted" };
    if (status?.state !== "on") return null;
    // Without the wireguard tools there's nothing to tell beyond "up"
    if (!peer) return { text: "On", tone: "text-ok" };
    if (answering) return { text: "Connected", tone: "text-ok" };
    return peer.handshake_age_secs == null
      ? { text: "On, but the server hasn't answered yet", tone: "text-warn" }
      : { text: "On, but the server has stopped answering", tone: "text-warn" };
  });
</script>

<Card title="WireGuard">
  {#if !status}
    <p class="px-4 py-3 text-[13px] text-muted">Loading…</p>
  {:else}
    {#if summary}
      <p class="px-4 py-3 text-[13px] {summary.tone}">{summary.text}</p>
    {/if}

    {#if status.state === "on"}
      <dl>
        {#if status.address}
          {@render row("Address", status.address.replace(/\/\d+$/, ""))}
        {/if}
        {#if peer?.endpoint}
          {@render row("Server", peer.endpoint)}
        {/if}
        {#if peer}
          {@render row(
            "Last answer",
            peer.handshake_age_secs == null
              ? "Never"
              : ago(peer.handshake_age_secs),
          )}
          {@render row(
            "Received / sent",
            `${bytes(peer.rx_bytes)} / ${bytes(peer.tx_bytes)}`,
          )}
        {/if}
      </dl>

      {#if peer && !peer.keepalive}
        <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
          The configuration has no <code class="font-mono"
            >PersistentKeepalive</code
          >. This device sits behind a router, so once the line has been quiet
          the server can't reach it any more. Add
          <code class="font-mono">PersistentKeepalive = 25</code> under
          <code class="font-mono">[Peer]</code> and upload it again.
        </p>
      {/if}
      {#if everything}
        <p class="border-t border-line px-4 py-3 text-[12px] text-muted">
          This configuration sends all of the device's internet traffic through
          the tunnel, including RTMP and SRT streams it pushes out.
        </p>
      {/if}
    {/if}

    <div
      class="flex flex-wrap items-center gap-2 border-t border-line px-4 py-3"
    >
      {#if status.state === "on"}
        <button
          type="button"
          class={btn.plain}
          disabled={busy}
          onclick={() => run(() => api.wireguardAction("disconnect"))}
        >
          {busy ? "Working…" : "Switch off"}
        </button>
      {:else if status.state === "off"}
        <button
          type="button"
          class={btn.plain}
          disabled={busy}
          onclick={() => run(() => api.wireguardAction("connect"))}
        >
          {busy ? "Working…" : "Switch on"}
        </button>
      {/if}
      <button
        type="button"
        class={status.state === "none" ? btn.plain : btn.text}
        disabled={busy}
        onclick={() => input?.click()}
      >
        {status.state === "none"
          ? busy
            ? "Working…"
            : "Upload configuration"
          : "Replace configuration"}
      </button>
      {#if status.state !== "none"}
        <button
          type="button"
          class={btn.text}
          disabled={busy}
          onclick={() => (confirmRemove = true)}
        >
          Remove
        </button>
      {/if}
    </div>
    <input
      bind:this={input}
      type="file"
      accept=".conf,text/plain"
      class="hidden"
      onchange={picked}
    />
  {/if}
</Card>

<ConfirmDialog
  bind:open={confirmRemove}
  title="Remove the WireGuard configuration?"
  message="The tunnel goes down and its keys are deleted from this device. If you're reaching this page through it, you'll lose it."
  confirmLabel="Remove"
  danger
  onconfirm={() =>
    run(() => api.wireguardRemove(), "WireGuard configuration removed")}
/>

{#snippet row(k: string, v: string)}
  <div
    class="flex items-baseline justify-between gap-4 border-t border-line px-4 py-2.5"
  >
    <dt class="text-[13px] text-fg">{k}</dt>
    <dd class="truncate text-right font-mono text-[13px] text-muted select-all">
      {v}
    </dd>
  </div>
{/snippet}
