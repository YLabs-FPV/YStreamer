<script lang="ts">
  import { bytes, duration, recording } from "@/stores/recording.svelte";

  const status = $derived(recording.status);
</script>

<section
  class="overflow-hidden rounded-xl border border-line bg-surface shadow-sm"
>
  <header
    class="flex items-center justify-between border-b border-line bg-surface-2/40 px-4 py-3"
  >
    <h2 class="text-[13px] font-medium text-fg">
      Recorder <span class="font-normal text-muted">· on this device</span>
    </h2>
    <a href="/recording/files" class="text-[12px] text-primary hover:underline">
      Files
    </a>
  </header>

  <div class="flex items-center gap-4 px-4 py-4">
    <button
      type="button"
      onclick={() => recording.toggle()}
      disabled={recording.busy}
      title={recording.active
        ? "Stop recording (Shift+R)"
        : "Start recording (Shift+R)"}
      aria-label={recording.active ? "Stop recording" : "Start recording"}
      class="group grid h-12 w-12 shrink-0 place-items-center rounded-full border-2 transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:opacity-50
        {recording.active
        ? 'border-down bg-down/10 hover:bg-down/20'
        : 'border-line-strong hover:border-down'}"
    >
      {#if recording.active}
        <span class="h-4 w-4 rounded-[3px] bg-down"></span>
      {:else}
        <span
          class="h-7 w-7 rounded-full bg-down transition-transform group-hover:scale-105"
        ></span>
      {/if}
    </button>

    <div class="min-w-0 flex-1">
      {#if recording.active}
        <p class="flex items-center gap-2">
          <span class="h-2 w-2 animate-pulse rounded-full bg-down"></span>
          <span class="font-mono text-xl text-fg tabular-nums">
            {duration(recording.elapsed)}
          </span>
        </p>
        <p class="mt-0.5 truncate text-[12px] text-muted">
          {bytes(status?.bytes ?? 0)}
        </p>
      {:else}
        <p class="text-[13px] text-fg">Not recording</p>
        {#if status?.storage}
          <p class="mt-0.5 truncate text-[12px] text-muted">
            {bytes(status.storage.free_bytes)} free
          </p>
        {/if}
      {/if}
    </div>
  </div>

  {#if status?.error}
    <p class="border-t border-line px-4 py-2.5 text-[12px] text-down">
      {status.error}
    </p>
  {/if}
</section>
