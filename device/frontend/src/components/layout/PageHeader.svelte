<script lang="ts">
  import type { Snippet } from "svelte";

  let { title, children }: { title: string; children?: Snippet } = $props();
</script>

<header class="flex flex-wrap items-center justify-between gap-3 pb-5">
  <h1 class="text-2xl font-semibold tracking-tight text-fg">{title}</h1>
  {@render children?.()}
</header>
