<script lang="ts">
  import Minus from "@lucide/svelte/icons/minus";
  import Plus from "@lucide/svelte/icons/plus";

  interface Props {
    label: string;
    value: number | null;
    values?: number[];
    min?: number;
    max?: number;
    step?: number;
    format?: (v: number) => string;
    minLabel?: string;
    maxLabel?: string;
    ticks?: string[];
    disabled?: boolean;
    note?: string;
    gradient?: string;
    onchange: (v: number) => void;
  }

  let {
    label,
    value,
    values,
    min = 0,
    max = 100,
    step = 1,
    format = (v: number) => String(v),
    minLabel,
    maxLabel,
    ticks,
    disabled = false,
    note,
    gradient,
    onchange: commit,
  }: Props = $props();

  const discrete = $derived(values !== undefined);
  const lo = $derived(discrete ? 0 : min);
  const hi = $derived(discrete ? values!.length - 1 : max);
  const stp = $derived(discrete ? 1 : step);

  let dragPos = $state<number | null>(null);

  function posOf(v: number | null): number {
    if (discrete) {
      const i = values!.indexOf(v as number);
      return i < 0 ? 0 : i;
    }
    if (v === null) return lo;
    return Math.min(hi, Math.max(lo, v));
  }
  const valOf = (p: number): number => (discrete ? values![p] : p);

  const pos = $derived(dragPos ?? posOf(value));
  const pct = $derived(hi === lo ? 0 : ((pos - lo) / (hi - lo)) * 100);
  const shown = $derived(
    value !== null || dragPos !== null ? format(valOf(pos)) : "—",
  );

  function nudge(dir: number) {
    const p = Math.min(hi, Math.max(lo, pos + dir * stp));
    if (p === pos) return;
    dragPos = null;
    commit(valOf(p));
  }
</script>

<div class="flex flex-col gap-0.5" class:opacity-45={disabled}>
  <div class="flex items-baseline justify-between gap-3">
    <span class="text-[11px] tracking-wide text-faint uppercase">{label}</span>
    <span class="font-mono text-[13px] text-fg tabular-nums">{shown}</span>
  </div>

  <div class="flex items-center gap-2">
    <button
      type="button"
      {disabled}
      onclick={() => nudge(-1)}
      aria-label="{label} down"
      class="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-line bg-surface-2 text-muted transition-colors hover:bg-surface-3 hover:text-fg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:pointer-events-none"
    >
      <Minus class="h-3.5 w-3.5" aria-hidden="true" />
    </button>

    <input
      type="range"
      class="range {gradient ? 'range--flat' : ''}"
      min={lo}
      max={hi}
      step={stp}
      value={pos}
      {disabled}
      aria-label={label}
      style="--pct: {pct}%{gradient ? `; --range-track: ${gradient}` : ''}"
      oninput={(e) => (dragPos = +e.currentTarget.value)}
      onchange={(e) => {
        const p = +e.currentTarget.value;
        dragPos = null;
        commit(valOf(p));
      }}
    />

    <button
      type="button"
      {disabled}
      onclick={() => nudge(1)}
      aria-label="{label} up"
      class="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-line bg-surface-2 text-muted transition-colors hover:bg-surface-3 hover:text-fg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:pointer-events-none"
    >
      <Plus class="h-3.5 w-3.5" aria-hidden="true" />
    </button>
  </div>

  {#if ticks}
    <div class="flex justify-between px-9 text-[10px] text-faint tabular-nums">
      {#each ticks as t}<span>{t}</span>{/each}
    </div>
  {:else if minLabel || maxLabel}
    <div class="flex justify-between px-9 text-[10px] text-faint tabular-nums">
      <span>{minLabel ?? ""}</span><span>{maxLabel ?? ""}</span>
    </div>
  {/if}

  {#if note}
    <p class="px-9 text-[11px] text-faint">{note}</p>
  {/if}
</div>
