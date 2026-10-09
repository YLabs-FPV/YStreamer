<script lang="ts">
  type Val = string | number | boolean;

  interface Props {
    label?: string;
    /** An option with a `blocked` reason is greyed out and can't be picked */
    options: { label: string; value: Val; blocked?: string | null }[];
    value: Val | null;
    disabled?: boolean;
    onchange: (v: any) => void;
    /** Told the reason when a greyed-out option is clicked anyway */
    onblocked?: (reason: string) => void;
  }

  let {
    label,
    options,
    value,
    disabled = false,
    onchange: commit,
    onblocked,
  }: Props = $props();

  const idx = $derived(options.findIndex((o) => o.value === value));
</script>

<div class="flex flex-col gap-1.5" class:opacity-45={disabled}>
  {#if label}
    <span class="text-[11px] tracking-wide text-faint uppercase">{label}</span>
  {/if}

  <div
    class="relative grid gap-0 rounded-lg border border-line bg-surface-2 p-1"
    style="grid-template-columns: repeat({options.length}, minmax(0, 1fr))"
  >
    <div
      class="pointer-events-none absolute top-1 bottom-1 left-1 rounded-md bg-primary shadow-sm transition-all duration-200 ease-out"
      class:opacity-0={idx < 0}
      style="width: calc((100% - 0.5rem) / {options.length}); transform: translateX({idx <
      0
        ? 0
        : idx * 100}%)"
    ></div>

    {#each options as o}
      <button
        type="button"
        {disabled}
        onclick={() => (o.blocked ? onblocked?.(o.blocked) : commit(o.value))}
        aria-pressed={o.value === value}
        aria-disabled={o.blocked ? true : undefined}
        title={o.blocked ?? undefined}
        class="relative z-10 rounded-md px-2 py-1.5 text-[13px] font-medium tabular-nums transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:pointer-events-none
          {o.value === value
          ? 'text-white'
          : o.blocked
            ? 'cursor-not-allowed text-faint/50 line-through'
            : 'text-muted hover:text-fg'}"
      >
        {o.label}
      </button>
    {/each}
  </div>
</div>
