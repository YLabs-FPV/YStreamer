<script lang="ts">
  import type { Snippet } from "svelte";
  import Brand from "@/components/ui/Brand.svelte";

  let { children }: { children: Snippet } = $props();
</script>

<div class="grid min-h-screen place-items-center px-4 py-10">
  <div class="flex w-full flex-col items-center gap-6">
    <Brand />
    {@render children()}
  </div>
</div>
