<script lang="ts">
  interface Props {
    percent: number | null;
  }
  let { percent }: Props = $props();

  const p = $derived(
    percent === null ? 0 : Math.min(100, Math.max(0, percent)),
  );

  const tone = $derived(p <= 15 ? "bg-down" : p <= 30 ? "bg-warn" : "bg-ok");
</script>

{#if percent !== null}
  <div
    class="flex items-center gap-1.5"
    title="Goggles battery {p}%"
    aria-label="Goggles battery {p}%"
  >
    <span
      class="relative flex h-3 w-6 items-center rounded-[3px] border border-line-strong p-[1.5px]"
    >
      <span
        class="h-full rounded-[1px] transition-all duration-500 {tone}"
        style="width: {p}%"
      ></span>
      <span
        class="absolute top-1/2 -right-0.75 h-1.5 w-0.5 -translate-y-1/2 rounded-r-[1px] bg-line-strong"
      ></span>
    </span>
    <span class="font-mono text-[12px] text-muted tabular-nums">{p}%</span>
  </div>
{/if}
