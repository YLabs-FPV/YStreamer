<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import type { ButtonStatus } from "@/types/recording";

  /** Where the button reports from, polled every second */
  let { load }: { load: () => Promise<ButtonStatus> } = $props();

  let status = $state<ButtonStatus | null>(null);
  let seen = $state<number | null>(null);
  let pressedAt = $state(0);
  let now = $state(Date.now());
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh() {
    now = Date.now();
    try {
      status = await load();
      if (seen !== null && status.presses > seen) pressedAt = now;
      seen = status.presses;
    } catch {
      // Shown as offline elsewhere
    }
  }

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 1000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  const justPressed = $derived(now - pressedAt < 3000);
</script>

{#if status?.error}
  <p class="bg-down/10 px-4 py-3 text-[13px] text-down">{status.error}</p>
{:else if status?.ready}
  <p class="px-4 py-3 text-[12px] {justPressed ? 'text-ok' : 'text-muted'}">
    {justPressed
      ? "Received."
      : "Listening. Press the button or flip the switch to see it register here."}
  </p>
{/if}
