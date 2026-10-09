<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, message } from "@/api/client";
  import type { TailscaleStatus } from "@/types/tailscale";
  import { auth } from "@/stores/auth.svelte";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";

  const INSTALL = "curl -fsSL https://tailscale.com/install.sh | sh";

  let status = $state<TailscaleStatus | null>(null);
  let busy = $state(false);
  let confirmLogout = $state(false);
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh() {
    if (busy) return;
    try {
      status = await api.tailscale();
    } catch {
      // Shown as offline elsewhere
    }
  }

  async function act(action: "connect" | "disconnect" | "logout") {
    busy = true;
    try {
      status = await api.tailscaleAction(action);
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 3000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  const phase = $derived(status?.state);
  const ipv4 = $derived(status?.addresses.find((a) => !a.includes(":")));
  const port = location.port ? `:${location.port}` : "";

  const summary = $derived.by(() => {
    switch (phase) {
      case "connected":
        return { text: "Connected", tone: "text-ok" };
      case "connecting":
        return { text: "Connecting…", tone: "text-warn" };
      case "needs_approval":
        return {
          text: "Signed in. Waiting for an admin of your Tailscale network to approve this device.",
          tone: "text-warn",
        };
      case "needs_login":
        return status?.login_url
          ? { text: "Waiting for you to sign in…", tone: "text-warn" }
          : { text: "Not set up", tone: "text-muted" };
      case "disconnected":
        return { text: "Signed in, but disconnected", tone: "text-muted" };
      default:
        return null;
    }
  });
</script>

<Card title="Tailscale">
  {#if !status}
    <p class="px-4 py-3 text-[13px] text-muted">Loading…</p>
  {:else if phase === "not_installed"}
    <div class="flex flex-col gap-2 px-4 py-3 text-[13px] text-fg">
      <p>Tailscale isn't installed on this device. Over SSH, run:</p>
      <code
        class="block overflow-x-auto rounded-md bg-surface-2 px-3 py-2 font-mono text-[12px] whitespace-nowrap select-all"
        >{INSTALL}</code
      >
      <p class="text-[12px] text-muted">Then come back to this page.</p>
    </div>
  {:else if phase === "service_down"}
    <div class="flex flex-col gap-2 px-4 py-3 text-[13px] text-fg">
      <p>
        Tailscale is installed, but its service isn't running. Over SSH, run:
      </p>
      <code
        class="block overflow-x-auto rounded-md bg-surface-2 px-3 py-2 font-mono text-[12px] whitespace-nowrap select-all"
        >sudo systemctl enable --now tailscaled</code
      >
    </div>
  {:else}
    {#if summary}
      <p class="px-4 py-3 text-[13px] {summary.tone}">{summary.text}</p>
    {/if}

    {#if phase === "needs_login" && status.login_url}
      <div class="flex flex-col gap-2 border-t border-line px-4 py-3">
        <p class="text-[13px] text-fg">
          Open this link on any device and sign in. This page follows along by
          itself.
        </p>
        <a
          href={status.login_url}
          target="_blank"
          rel="noopener"
          class="font-mono text-[13px] break-all text-primary underline"
          >{status.login_url}</a
        >
      </div>
    {/if}

    {#if phase === "connected"}
      <dl>
        {#if status.name}
          {@render row("Name", status.name)}
        {/if}
        {#if ipv4}
          {@render row("Address", ipv4)}
        {/if}
        {#if status.network}
          {@render row("Network", status.network)}
        {/if}
        {#if ipv4}
          {@render row("This page", `http://${status.name ?? ipv4}${port}`)}
        {/if}
      </dl>

      {#if status.peers.length}
        <div class="border-t border-line px-4 py-3">
          <p class="text-[11px] tracking-wide text-faint uppercase">
            In use by
          </p>
          <ul class="mt-1.5 flex flex-col gap-1">
            {#each status.peers as peer}
              <li class="flex items-baseline justify-between gap-4 text-[13px]">
                <span class="truncate text-fg">{peer.name}</span>
                <span class={peer.direct ? "text-ok" : "text-warn"}>
                  {peer.direct
                    ? "Direct"
                    : `Relayed${peer.relay ? ` via ${peer.relay}` : ""}, adds delay`}
                </span>
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if !auth.enabled}
        <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
          No password is set, so everyone on your Tailscale network can change
          these settings. Set one under System → General.
        </p>
      {/if}
    {/if}

    {#each status.problems as problem}
      <p class="border-t border-line px-4 py-3 text-[12px] text-warn">
        {problem}
      </p>
    {/each}

    <div
      class="flex flex-wrap items-center gap-2 border-t border-line px-4 py-3"
    >
      {#if phase === "connected" || phase === "connecting"}
        <button
          type="button"
          class={btn.plain}
          disabled={busy}
          onclick={() => act("disconnect")}
        >
          {busy ? "Working…" : "Disconnect"}
        </button>
      {:else if !(phase === "needs_login" && status.login_url)}
        <button
          type="button"
          class={btn.plain}
          disabled={busy}
          onclick={() => act("connect")}
        >
          {busy ? "Working…" : phase === "needs_login" ? "Set up" : "Connect"}
        </button>
      {/if}
      {#if phase !== "needs_login"}
        <button
          type="button"
          class={btn.text}
          disabled={busy}
          onclick={() => (confirmLogout = true)}
        >
          Sign out
        </button>
      {/if}
    </div>
  {/if}
</Card>

<ConfirmDialog
  bind:open={confirmLogout}
  title="Sign out of Tailscale?"
  message="This device leaves your Tailscale network. If you're reaching this page through Tailscale, you'll lose it, and setting it up again needs a connection from the local network."
  confirmLabel="Sign out"
  danger
  onconfirm={() => act("logout")}
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
