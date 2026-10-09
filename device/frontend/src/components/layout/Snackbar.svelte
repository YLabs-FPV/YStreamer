<script lang="ts">
  import { snackbar } from "@/stores/snackbar.svelte";
  import X from "@lucide/svelte/icons/x";

  // Raised to clear the unsaved-changes bar
  let { raised = false }: { raised?: boolean } = $props();
</script>

{#if snackbar.message}
  <div
    role="status"
    class="fixed right-0 left-0 z-30 flex justify-center px-4 md:left-56 {raised
      ? 'bottom-20'
      : 'bottom-5'}"
  >
    <div
      class="flex max-w-lg items-center gap-2 rounded-lg py-1.5 pr-1.5 pl-4 text-[13px] shadow-lg
        {snackbar.error ? 'bg-down text-white' : 'bg-fg text-surface'}"
    >
      <span class="py-1">{snackbar.message}</span>
      <button
        type="button"
        onclick={() => snackbar.dismiss()}
        aria-label="Dismiss"
        title="Dismiss"
        class="ml-2 grid h-7 w-7 shrink-0 place-items-center rounded-md bg-current/10 transition-colors hover:bg-current/25 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-current"
      >
        <X class="h-3.5 w-3.5" aria-hidden="true" />
      </button>
    </div>
  </div>
{/if}
