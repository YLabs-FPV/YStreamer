<script lang="ts">
  let { quality }: { quality: number | null } = $props();

  const BARS = 5;
  // Reported in steps of ten, and 90 is as good as it gets
  const lit = $derived(
    quality === null ? 0 : Math.min(BARS, Math.max(0, Math.ceil(quality / 20))),
  );
  const tone = $derived(lit <= 1 ? "bg-down" : lit <= 3 ? "bg-warn" : "bg-ok");
</script>

{#if quality !== null}
  <div
    class="flex h-3 items-end gap-[2px]"
    title="Video link {lit} of {BARS}"
    aria-label="Video link {lit} of {BARS} bars"
    role="img"
  >
    {#each { length: BARS } as _, i}
      <span
        class="w-[3px] rounded-[1px] transition-colors {i < lit
          ? tone
          : 'bg-line-strong'}"
        style="height: {((i + 1) / BARS) * 100}%"
      ></span>
    {/each}
  </div>
{/if}
