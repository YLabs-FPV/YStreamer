<script lang="ts">
  import X from "@lucide/svelte/icons/x";
  import type { PlayerHistory, PlayerStat } from "@/types/stream";
  import Chart from "@/components/ui/Chart.svelte";

  let {
    stats,
    history,
    onclose,
  }: { stats: PlayerStat[]; history: PlayerHistory; onclose: () => void } =
    $props();

  const rate = (kbps: number) =>
    kbps >= 1000
      ? `${(kbps / 1000).toFixed(1)} Mb/s`
      : `${Math.round(kbps)} kb/s`;
  const millis = (v: number) => `${Math.round(v)} ms`;
</script>

<section
  class="@container overflow-hidden rounded-xl border border-line bg-surface shadow-sm"
>
  <header
    class="flex items-center justify-between border-b border-line bg-surface-2/40 px-4 py-2"
  >
    <h2 class="text-[13px] font-medium text-fg">Stats</h2>
    <button
      type="button"
      onclick={onclose}
      aria-label="Hide stats"
      class="grid h-7 w-7 place-items-center rounded-md text-muted transition-colors hover:bg-surface-2 hover:text-fg focus-visible:outline-2 focus-visible:outline-primary"
    >
      <X class="h-4 w-4" aria-hidden="true" />
    </button>
  </header>

  <dl class="flex flex-wrap gap-x-6 gap-y-2 border-b border-line px-4 py-3">
    {#each stats as s}
      <div>
        <dt class="text-[11px] tracking-wide text-faint uppercase">
          {s.label}
        </dt>
        <dd class="font-mono text-[13px] text-fg tabular-nums">{s.v}</dd>
      </div>
    {/each}
  </dl>

  <div
    class="grid divide-y divide-line @lg:grid-cols-2 @lg:divide-x @lg:divide-y-0"
  >
    <Chart
      t={history.t}
      windowSecs={120}
      format={rate}
      series={[
        {
          label: "Bitrate",
          color: "var(--color-primary)",
          values: history.kbps,
          fill: true,
        },
      ]}
    />
    <Chart
      t={history.t}
      windowSecs={120}
      format={millis}
      series={[
        {
          label: "Buffer",
          color: "var(--color-accent)",
          values: history.bufferMs,
          fill: true,
        },
      ]}
    />
  </div>
</section>
