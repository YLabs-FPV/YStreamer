<script lang="ts">
  import ChevronDown from "@lucide/svelte/icons/chevron-down";

  type Val = string | number;

  interface Props {
    value: Val;
    options: { label: string; value: Val }[];
    id?: string;
    disabled?: boolean;
    class?: string;
    onchange?: (value: Val) => void;
  }

  let {
    value = $bindable(),
    options,
    id,
    disabled = false,
    class: extra = "",
    onchange,
  }: Props = $props();
</script>

<div class="relative w-full sm:w-64 {extra}">
  <select
    {id}
    {disabled}
    bind:value
    onchange={() => onchange?.(value)}
    class="w-full appearance-none rounded-md border border-line bg-surface-2 py-1.5 pr-8 pl-3 text-[13px] text-fg transition-colors outline-none hover:border-line-strong focus:border-primary disabled:pointer-events-none"
  >
    {#each options as o (o.value)}
      <option value={o.value}>{o.label}</option>
    {/each}
  </select>
  <ChevronDown
    class="pointer-events-none absolute top-1/2 right-2.5 h-3.5 w-3.5 -translate-y-1/2 text-faint"
    aria-hidden="true"
  />
</div>
