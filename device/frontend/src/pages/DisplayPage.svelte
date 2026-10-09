<script lang="ts">
  import type { Settings } from "@/types/settings";
  import HdmiCard from "@/components/display/HdmiCard.svelte";
  import SplashCard from "@/components/display/SplashCard.svelte";
  import OverlayCard from "@/components/display/OverlayCard.svelte";
  let {
    display = $bindable(),
    splash = $bindable(),
    overlay = $bindable(),
    tab,
  }: {
    display: Settings["display"];
    splash: Settings["splash"];
    overlay: Settings["overlay"];
    tab: string;
  } = $props();
</script>

<div class="flex flex-col gap-4">
  {#if tab === "hdmi"}
    <HdmiCard bind:display />
  {:else if tab === "splash"}
    <SplashCard bind:splash />
  {:else if tab === "logo"}
    <OverlayCard bind:overlay />
  {/if}
</div>
