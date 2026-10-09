<script lang="ts">
  import { control } from "@/stores/control.svelte";
  import { duration } from "@/stores/recording.svelte";
  import type { AircraftStorage } from "@/types/duml";

  const unit = $derived(control.aircraftRecorder);

  // Every unit reports the storage it records to. The O4 units also list
  // their storages by kind; the O3 doesn't, and which of the two its
  // figures are for can only be guessed, so it isn't
  const split = $derived(unit?.sd != null || unit?.internal != null);

  const size = (mb: number) =>
    mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
  const timeLeft = (secs: number) =>
    secs >= 3600
      ? `${Math.floor(secs / 3600)} h ${Math.round((secs % 3600) / 60)} min`
      : secs >= 60
        ? `${Math.round(secs / 60)} min`
        : `${secs} s`;
</script>

{#if unit}
  <section
    class="overflow-hidden rounded-xl border border-line bg-surface shadow-sm"
  >
    <header
      class="flex items-center justify-between gap-3 border-b border-line bg-surface-2/40 px-4 py-3"
    >
      <h2 class="truncate text-[13px] font-medium text-fg">
        {control.aircraft?.name ?? "Air unit"}
        <span class="font-normal text-muted">· onboard recording</span>
      </h2>
      {#if unit.recording}
        <span class="flex shrink-0 items-center gap-1.5">
          <span class="h-2 w-2 animate-pulse rounded-full bg-down"></span>
          <span class="font-mono text-[13px] text-fg tabular-nums">
            {duration(unit.record_secs)}
          </span>
        </span>
      {:else}
        <span class="shrink-0 text-[12px] text-muted">Not recording</span>
      {/if}
    </header>

    <div class="divide-y divide-line">
      {#if split}
        {#if unit.sd}
          {@render storage("SD card", unit.sd, !unit.to_internal)}
        {:else if unit.has_sd_slot}
          <p class="px-4 py-3 text-[12px] text-muted">No SD card</p>
        {/if}
        {#if unit.internal}
          {@render storage(
            "Internal",
            unit.internal,
            unit.has_sd_slot && unit.to_internal === true,
          )}
        {/if}
      {:else if unit.storage}
        {@render storage("Storage", unit.storage, false)}
      {/if}
    </div>
  </section>
{/if}

{#snippet storage(label: string, s: AircraftStorage, target: boolean)}
  {@const used = s.total_mb ? ((s.total_mb - s.free_mb) / s.total_mb) * 100 : 0}
  {@const low = s.secs_left < 60}
  <div class="px-4 py-3">
    <div class="flex items-baseline justify-between gap-3 text-[12px]">
      <span class="flex items-center gap-2 text-fg">
        {label}
        {#if target}
          <span
            class="rounded-full bg-surface-2 px-1.5 py-0.5 text-[10px] tracking-wide text-muted uppercase"
            >Records here</span
          >
        {/if}
      </span>
      <span class="{low ? 'text-down' : 'text-muted'} tabular-nums">
        {#if s.secs_left === 0}
          Full
        {:else}
          {timeLeft(s.secs_left)} left
        {/if}
      </span>
    </div>
    <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-surface-3">
      <div
        class="h-full {low ? 'bg-down' : 'bg-primary'}"
        style="width: {used}%"
      ></div>
    </div>
    <p class="mt-1.5 font-mono text-[11px] text-muted tabular-nums">
      {size(s.free_mb)} free of {size(s.total_mb)}
    </p>
  </div>
{/snippet}
