<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    label: string;
    help?: string;
    for?: string;
    stacked?: boolean;
    disabled?: boolean;
    children: Snippet;
  }

  let {
    label,
    help,
    for: htmlFor,
    stacked = false,
    disabled = false,
    children,
  }: Props = $props();
</script>

<div
  class="flex gap-x-6 gap-y-2 px-4 py-3 {stacked
    ? 'flex-col'
    : 'flex-col sm:flex-row sm:items-center sm:justify-between'}"
  class:opacity-45={disabled}
>
  <div class="min-w-0 sm:max-w-[55%]">
    <label for={htmlFor} class="block text-[13px] text-fg">{label}</label>
    {#if help}
      <p class="mt-0.5 text-[12px] leading-snug text-muted">{help}</p>
    {/if}
  </div>
  <div
    class="flex shrink-0 items-center gap-2 {stacked ? '' : 'sm:justify-end'}"
  >
    {@render children()}
  </div>
</div>
