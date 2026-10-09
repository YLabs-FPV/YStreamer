<script lang="ts">
  interface Props {
    checked: boolean;
    id?: string;
    label?: string;
    disabled?: boolean;
    busy?: boolean;
  }

  let {
    checked = $bindable(),
    id,
    label,
    disabled = false,
    busy = false,
  }: Props = $props();
</script>

<button
  {id}
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  disabled={disabled || busy}
  aria-busy={busy}
  onclick={() => (checked = !checked)}
  class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:pointer-events-none
    {checked ? 'bg-primary' : 'bg-surface-3'}"
>
  <span
    class="inline-flex h-4 w-4 items-center justify-center rounded-full bg-white shadow-sm transition-transform duration-150
      {checked ? 'translate-x-4.5' : 'translate-x-0.5'}"
  >
    {#if busy}
      <span
        class="h-2.5 w-2.5 animate-spin rounded-full border-[1.5px] border-primary border-t-transparent"
      ></span>
    {/if}
  </span>
</button>
