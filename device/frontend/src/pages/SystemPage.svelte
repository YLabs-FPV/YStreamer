<script lang="ts">
  import type { Settings } from "@/types/settings";
  import DeviceSection from "@/components/system/DeviceSection.svelte";
  import MaintenanceSection from "@/components/system/MaintenanceSection.svelte";
  let {
    device = $bindable(),
    advanced = $bindable(),
    tab,
  }: {
    device: Settings["device"];
    advanced: Settings["advanced"];
    tab: string;
  } = $props();
</script>

{#if tab === "general" || tab === "about"}
  <DeviceSection bind:device {tab} />
{:else}
  <MaintenanceSection bind:advanced {tab} />
{/if}
