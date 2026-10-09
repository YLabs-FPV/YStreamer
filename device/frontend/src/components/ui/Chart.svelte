<script lang="ts">
  export interface ChartSeries {
    label: string;
    color: string;
    values: (number | null)[];
    fill?: boolean;
  }

  interface Props {
    /** When each value was taken, Unix milliseconds */
    t: number[];
    series: ChartSeries[];
    windowSecs: number;
    max?: number;
    min?: number;
    format?: (v: number) => string;
  }

  let {
    t,
    series,
    windowSecs,
    max,
    min = 0,
    format = (v: number) => String(Math.round(v)),
  }: Props = $props();

  const HEIGHT = 120;

  let width = $state(0);
  let hover = $state<number | null>(null);

  const end = $derived(t.length ? t[t.length - 1] : 0);
  const start = $derived(end - windowSecs * 1000);
  // Index of the first value inside the window
  const from = $derived.by(() => {
    let lo = 0;
    let hi = t.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (t[mid] < start) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  });

  // A round number comfortably above the highest value in view
  function ceiling(v: number) {
    if (v <= 0) return 1;
    const magnitude = 10 ** Math.floor(Math.log10(v));
    for (const step of [1, 2, 5, 10])
      if (v <= step * magnitude) return step * magnitude;
    return 10 * magnitude;
  }

  const top = $derived.by(() => {
    if (max !== undefined) return max;
    let highest = 0;
    for (const s of series)
      for (let i = from; i < s.values.length; i++) {
        const v = s.values[i];
        if (v !== null && v - min > highest) highest = v - min;
      }
    return min + ceiling(highest * 1.1);
  });

  const x = (ms: number) => ((ms - start) / (windowSecs * 1000)) * width;
  const y = (v: number) =>
    HEIGHT -
    ((Math.min(Math.max(v, min), top) - min) / (top - min || 1)) * HEIGHT;

  // One path per series, lifting the pen over gaps
  const paths = $derived(
    series.map((s) => {
      let line = "";
      let area = "";
      let runStart: number | null = null;
      let last = 0;
      const close = () => {
        if (runStart !== null && s.fill)
          area += `L${last.toFixed(1)},${HEIGHT}L${runStart.toFixed(1)},${HEIGHT}Z`;
        runStart = null;
      };
      for (let i = from; i < t.length; i++) {
        const v = s.values[i];
        if (v === null || v === undefined) {
          close();
          continue;
        }
        const px = x(t[i]);
        const point = `${px.toFixed(1)},${y(v).toFixed(1)}`;
        if (runStart === null) {
          line += `M${point}`;
          if (s.fill) area += `M${point}`;
          runStart = px;
        } else {
          line += `L${point}`;
          if (s.fill) area += `L${point}`;
        }
        last = px;
      }
      close();
      return { line, area };
    }),
  );

  function track(e: PointerEvent) {
    if (!t.length || !width) return;
    const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const ms = start + ((e.clientX - box.left) / box.width) * windowSecs * 1000;
    let best = from;
    for (let i = from; i < t.length; i++)
      if (Math.abs(t[i] - ms) < Math.abs(t[best] - ms)) best = i;
    hover = best < t.length ? best : null;
  }

  // What the legend reads out: the hovered moment, or right now
  const at = $derived(
    hover !== null && hover < t.length ? hover : t.length - 1,
  );
  const ago = $derived.by(() => {
    if (hover === null || !t.length) return null;
    const secs = Math.round((end - t[at]) / 1000);
    return secs < 60
      ? `${secs} s ago`
      : `${Math.floor(secs / 60)} min ${secs % 60} s ago`;
  });
</script>

<div class="flex flex-col gap-2 px-4 py-3">
  <div class="flex flex-wrap items-baseline gap-x-4 gap-y-1">
    {#each series as s}
      {@const v = s.values[at]}
      <span class="flex items-baseline gap-1.5 text-[12px]">
        <span
          class="h-2 w-2 shrink-0 self-center rounded-full"
          style="background:{s.color}"
        ></span>
        <span class="text-muted">{s.label}</span>
        <span class="font-mono text-fg tabular-nums">
          {v === null || v === undefined ? "—" : format(v)}
        </span>
      </span>
    {/each}
    {#if ago}
      <span class="ml-auto text-[11px] text-faint">{ago}</span>
    {/if}
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    bind:clientWidth={width}
    class="relative touch-none overflow-hidden rounded-md border border-line bg-surface-2/40"
    style="height:{HEIGHT}px"
    onpointermove={track}
    onpointerleave={() => (hover = null)}
  >
    {#if width}
      <svg {width} height={HEIGHT} class="block" aria-hidden="true">
        {#each [0.25, 0.5, 0.75] as line}
          <line
            x1="0"
            x2={width}
            y1={HEIGHT * line}
            y2={HEIGHT * line}
            class="stroke-line"
            stroke-width="1"
          />
        {/each}
        {#each paths as p, i}
          {#if p.area}
            <path d={p.area} fill={series[i].color} opacity="0.12" />
          {/if}
          <path
            d={p.line}
            fill="none"
            stroke={series[i].color}
            stroke-width="1.5"
            stroke-linejoin="round"
          />
        {/each}
        {#if hover !== null && hover < t.length}
          <line
            x1={x(t[hover])}
            x2={x(t[hover])}
            y1="0"
            y2={HEIGHT}
            class="stroke-line-strong"
            stroke-width="1"
          />
        {/if}
      </svg>
    {/if}
    <span
      class="pointer-events-none absolute top-1 left-1.5 text-[10px] text-faint tabular-nums"
    >
      {format(top)}
    </span>
    <span
      class="pointer-events-none absolute bottom-1 left-1.5 text-[10px] text-faint tabular-nums"
    >
      {format(min)}
    </span>
  </div>
</div>
