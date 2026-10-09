<script lang="ts">
  import type { Snippet } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { control } from "@/stores/control.svelte";
  import { route } from "@/stores/route.svelte";
  import Sidebar from "@/components/layout/Sidebar.svelte";
  import Battery from "@/components/ui/Battery.svelte";
  import SignalBars from "@/components/ui/SignalBars.svelte";
  import Brand from "@/components/ui/Brand.svelte";
  import Menu from "@lucide/svelte/icons/menu";

  let {
    current,
    width,
    children,
  }: { current: string; width: string; children: Snippet } = $props();

  let drawerOpen = $state(false);
  const closeDrawer = () => (drawerOpen = false);

  $effect(() => {
    route.path;
    closeDrawer();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") closeDrawer();
  }
</script>

<svelte:window {onkeydown} />

<div class="flex min-h-full">
  <aside
    class="sticky top-0 hidden h-screen w-56 shrink-0 border-r border-line bg-surface md:block"
  >
    <Sidebar {current} onnavigate={closeDrawer} />
  </aside>

  {#if drawerOpen}
    <div class="fixed inset-0 z-40 md:hidden">
      <button
        type="button"
        aria-label="Close menu"
        class="absolute inset-0 bg-black/40"
        onclick={closeDrawer}
        transition:fade={{ duration: 150 }}
      ></button>
      <aside
        class="absolute inset-y-0 left-0 w-64 max-w-[85%] border-r border-line bg-surface shadow-lg"
        transition:fly={{ x: -256, duration: 180, opacity: 1 }}
      >
        <Sidebar {current} onnavigate={closeDrawer} />
      </aside>
    </div>
  {/if}

  <div class="flex min-w-0 flex-1 flex-col">
    <div
      class="sticky top-0 z-30 flex h-14 items-center gap-2 border-b border-line bg-surface px-2 md:hidden"
    >
      <button
        type="button"
        onclick={() => (drawerOpen = true)}
        aria-label="Open menu"
        aria-expanded={drawerOpen}
        class="grid h-10 w-10 place-items-center rounded-md text-muted transition-colors hover:bg-surface-2 hover:text-fg focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary"
      >
        <Menu class="h-4 w-4" aria-hidden="true" />
      </button>
      <Brand small />
      <div class="mr-2 ml-auto flex items-center gap-3">
        <SignalBars quality={control.aircraft ? control.linkQuality : null} />
        <Battery percent={control.gogglesBattery} />
      </div>
    </div>

    <main class="mx-auto w-full px-4 py-6 sm:px-8 {width}">
      {@render children()}
    </main>
  </div>
</div>
