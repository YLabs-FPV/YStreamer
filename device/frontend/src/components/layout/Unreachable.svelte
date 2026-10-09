<script lang="ts">
  import { auth } from "@/stores/auth.svelte";
  import { btn } from "@/components/ui/buttons";
  import Unplug from "@lucide/svelte/icons/unplug";
</script>

<div
  role="alert"
  class="mx-auto flex w-full max-w-sm flex-col gap-4 rounded-xl border border-line bg-surface p-6 shadow-sm"
>
  <div class="flex items-center gap-3">
    <span
      class="grid h-9 w-9 shrink-0 place-items-center rounded-full bg-surface-2"
    >
      <Unplug class="h-4 w-4 text-muted" aria-hidden="true" />
    </span>
    <div class="min-w-0">
      <h2 class="text-[15px] font-semibold text-fg">Can't reach YStreamer</h2>
      <p class="truncate text-[12px] text-muted">{auth.unreachable}</p>
    </div>
  </div>
  <p class="text-[13px] text-muted">
    The device may be starting up or restarting. Trying again every few seconds.
  </p>
  <button type="button" class={btn.plain} onclick={() => auth.load()}>
    Retry now
  </button>
</div>
