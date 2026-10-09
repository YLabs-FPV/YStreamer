<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import { btn } from "@/components/ui/buttons";
  import Switch from "@/components/ui/Switch.svelte";

  const REFRESH_KEY = "ystreamer.logs.refresh";
  const REFRESH_MS = 1000;
  const BOTTOM_SLACK_PX = 4;

  function savedRefresh(): boolean {
    try {
      return localStorage.getItem(REFRESH_KEY) !== "0";
    } catch {
      return true;
    }
  }

  let logs = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let noSystemd = $state(false);
  let refresh = $state(savedRefresh());
  let logEl = $state<HTMLPreElement>();
  // New lines only pull the view down when it was at the bottom already
  let atBottom = true;

  /** `quiet` is the background refresh, which doesn't touch the button */
  async function load(quiet = false) {
    if (!quiet) loading = true;
    try {
      const next = await api.logs();
      if (logEl)
        atBottom =
          logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight <
          BOTTOM_SLACK_PX;
      // Unchanged text is left alone, so a selection survives
      if (next !== logs) logs = next;
      error = null;
    } catch (e) {
      // A missed background refresh isn't worth replacing the log with
      if (!quiet || logs === null) error = message(e);
    } finally {
      if (!quiet) loading = false;
    }
  }

  function setRefresh(on: boolean) {
    refresh = on;
    try {
      if (on) localStorage.removeItem(REFRESH_KEY);
      else localStorage.setItem(REFRESH_KEY, "0");
    } catch {}
    if (on) load(true);
  }

  $effect(() => {
    if (logEl && logs !== null && atBottom)
      logEl.scrollTop = logEl.scrollHeight;
  });

  $effect(() => {
    if (!refresh || noSystemd) return;
    const timer = setInterval(() => {
      if (!document.hidden) load(true);
    }, REFRESH_MS);
    return () => clearInterval(timer);
  });

  onMount(async () => {
    try {
      noSystemd = !(await api.system()).systemd;
    } catch {}
    if (!noSystemd) load();
  });
</script>

<div class="flex flex-col gap-4">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <p class="text-[12px] text-muted">
      The latest lines from the YStreamer service.
    </p>
    <div class="flex items-center gap-4">
      <label
        for="logs-refresh"
        class="flex items-center gap-2 text-[13px] text-muted"
      >
        Auto-refresh
        <Switch
          id="logs-refresh"
          disabled={noSystemd}
          bind:checked={() => refresh, (on) => setRefresh(on)}
        />
      </label>
      <button
        type="button"
        class={btn.plain}
        disabled={noSystemd || loading}
        onclick={() => load()}
      >
        {loading ? "Loading…" : "Refresh"}
      </button>
    </div>
  </div>

  {#if noSystemd}
    <p class="text-[13px] text-muted">
      Logs need YStreamer to run under systemd.
    </p>
  {:else if error}
    <p class="text-[13px] text-down">{error}</p>
  {:else if logs === null}
    <p class="text-[13px] text-muted">Loading…</p>
  {:else}
    <pre
      bind:this={logEl}
      class="h-[70vh] min-h-72 overflow-auto rounded-xl border border-line bg-surface p-4 font-mono text-[11px] leading-relaxed text-fg shadow-sm">{logs ||
        "No log lines yet."}</pre>
  {/if}
</div>
