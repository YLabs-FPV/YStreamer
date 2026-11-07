<script lang="ts">
  import { onDestroy } from "svelte";

  interface Props {
    label: string;
    disabled?: boolean;
    holdMs?: number;
    onconfirm: () => void;
  }

  let { label, disabled = false, holdMs = 1000, onconfirm }: Props = $props();

  let holding = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  // Fires once per press: holding on doesn't repeat it
  function start() {
    if (disabled || timer) return;
    holding = true;
    timer = setTimeout(() => {
      timer = null;
      holding = false;
      onconfirm();
    }, holdMs);
  }

  function stop() {
    if (timer) clearTimeout(timer);
    timer = null;
    holding = false;
  }

  const isKey = (e: KeyboardEvent) => e.key === " " || e.key === "Enter";

  $effect(() => {
    if (disabled) stop();
  });
  onDestroy(stop);
</script>

<button
  type="button"
  {disabled}
  aria-label="{label}, hold to confirm"
  title="Hold for a second"
  onpointerdown={(e) => {
    if (e.button === 0) start();
  }}
  onpointerup={stop}
  onpointerleave={stop}
  onpointercancel={stop}
  onkeydown={(e) => {
    if (!isKey(e)) return;
    e.preventDefault();
    if (!e.repeat) start();
  }}
  onkeyup={(e) => {
    if (isKey(e)) stop();
  }}
  onblur={stop}
  oncontextmenu={(e) => e.preventDefault()}
  class="relative overflow-hidden rounded-md border border-line bg-surface-2 px-3 py-2.5 text-[13px] font-medium text-fg transition-colors select-none hover:bg-surface-3 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:pointer-events-none disabled:opacity-45"
  style="touch-action: none"
>
  <span
    class="absolute inset-y-0 left-0 bg-primary/30"
    style="width: {holding ? 100 : 0}%; transition: width {holding
      ? holdMs
      : 150}ms linear"
  ></span>
  <span class="relative">{label}</span>
</button>
