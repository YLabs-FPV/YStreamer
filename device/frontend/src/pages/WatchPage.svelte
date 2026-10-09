<script lang="ts">
  import { onMount } from "svelte";
  import { auth } from "@/stores/auth.svelte";
  import { control } from "@/stores/control.svelte";
  import { recording } from "@/stores/recording.svelte";
  import type { PlayerHistory, PlayerStat, Status } from "@/types/stream";
  import Player from "@/components/watch/Player.svelte";
  import CameraPanel from "@/components/watch/CameraPanel.svelte";
  import RecorderWidget from "@/components/watch/RecorderWidget.svelte";
  import AircraftRecorderWidget from "@/components/watch/AircraftRecorderWidget.svelte";
  import StatsWidget from "@/components/watch/StatsWidget.svelte";
  import PageHeader from "@/components/layout/PageHeader.svelte";
  import { statusDot, statusLabel } from "@/lib/status";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";

  let {
    title,
    theater = $bindable(false),
  }: { title: string; theater?: boolean } = $props();

  let status = $state<Status>("connecting");
  let player = $state<Player>();
  let stats = $state<PlayerStat[]>([]);
  let history = $state.raw<PlayerHistory>({ t: [], kbps: [], bufferMs: [] });
  let showStats = $state(false);

  onMount(() => {
    if (auth.admin) recording.load();
  });

  const camera = $derived(auth.admin && control.inputMode === "dji_fpv");
  const aircraftRecorder = $derived(
    camera && control.aircraftRecorder !== null,
  );
  const side = $derived(auth.admin || showStats);

  function onkeydown(e: KeyboardEvent) {
    if (e.ctrlKey || e.metaKey || e.altKey || e.repeat) return;
    const el = e.target as HTMLElement | null;
    if (el?.closest("input, textarea, select, [contenteditable], dialog"))
      return;
    const key = e.key.toLowerCase();
    if (e.shiftKey !== (key === "r")) return;
    switch (key) {
      case "f":
        player?.toggleFullscreen();
        break;
      case "t":
        player?.toggleTheater();
        break;
      case "p":
        player?.togglePip();
        break;
      case "s":
        player?.toggleStats();
        break;
      case "r":
        if (auth.admin) recording.toggle();
        break;
      default:
        return;
    }
    e.preventDefault();
  }
</script>

<svelte:window {onkeydown} />

<PageHeader {title}>
  <div class="flex items-center gap-x-4">
    <span class="flex items-center gap-2 rounded-full px-2.5 py-1">
      <span class="relative flex h-1.5 w-1.5">
        {#if status === "playing"}
          <span
            class="absolute inline-flex h-full w-full rounded-full bg-ok opacity-70"
          ></span>
        {/if}
        <span
          class="relative inline-flex h-1.5 w-1.5 rounded-full {statusDot(
            status,
          )}"
        ></span>
      </span>
      <span class="text-[12px] font-medium text-muted"
        >{statusLabel(status)}</span
      >
    </span>
    <button
      onclick={() => player?.reconnect()}
      class="flex items-center gap-1.5 rounded-md border border-line bg-surface-2 px-3 py-1.5 text-[13px] font-medium text-muted transition-colors hover:bg-surface-3 hover:text-fg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
    >
      <RefreshCw class="h-3.5 w-3.5" aria-hidden="true" />
      Reconnect
    </button>
  </div>
</PageHeader>

<div class="grid gap-4 {side ? 'lg:grid-cols-[minmax(0,1fr)_22rem]' : ''}">
  <div
    class="player flex min-w-0 flex-col gap-4 {theater && side
      ? 'lg:col-span-2'
      : ''}"
  >
    <Player
      bind:this={player}
      bind:status
      bind:theater
      bind:showStats
      bind:stats
      bind:history
    />
  </div>

  {#if side}
    <!-- Beside the player and whatever is under it; in theater mode the
         player takes the whole first row and this starts on the second -->
    <aside
      class="side grid min-w-0 content-start gap-4 lg:col-start-2 {theater
        ? 'lg:row-start-2'
        : 'lg:row-span-2 lg:row-start-1'}"
    >
      {#if auth.admin}
        <RecorderWidget />
      {/if}
      {#if aircraftRecorder}
        <AircraftRecorderWidget />
      {/if}
      {#if showStats}
        <StatsWidget {stats} {history} onclose={() => player?.toggleStats()} />
      {/if}
    </aside>
  {/if}

  {#if camera}
    <div class="camera min-w-0 lg:col-start-1">
      <CameraPanel />
    </div>
  {/if}
</div>

<style>
  .player {
    view-transition-name: player;
  }
  .side {
    view-transition-name: stream-side;
  }
  .camera {
    view-transition-name: stream-camera;
  }
</style>
