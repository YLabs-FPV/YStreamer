<script lang="ts">
  import { copyText } from "@/lib/clipboard";
  import { auth } from "@/stores/auth.svelte";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import { btn } from "@/components/ui/buttons";

  let { on = $bindable(), saved }: { on: boolean; saved: boolean } = $props();

  // The address this page was opened at works for the drive too
  const address = $derived(`http://${location.host}/dav/`);

  async function copyAddress() {
    try {
      await copyText(address);
      snackbar.show("Address copied");
    } catch {
      snackbar.show(
        "Couldn't copy; select the address and copy it by hand",
        true,
      );
    }
  }
</script>

<Card
  title="Network drive"
  description="Open and copy the recordings from Finder, Explorer or a file manager, read-only."
>
  {#snippet aside()}
    <Switch bind:checked={on} label="Network drive" />
  {/snippet}

  {#if saved}
    <div class="flex flex-col gap-2 px-4 py-3">
      <div class="flex items-center gap-2">
        <code
          class="min-w-0 flex-1 truncate rounded-md bg-surface-2 px-3 py-1.5 font-mono text-[13px] text-fg select-all"
        >
          {address}
        </code>
        <button type="button" class={btn.plain} onclick={copyAddress}
          >Copy</button
        >
      </div>
      <p class="text-[12px] text-muted">
        {auth.enabled
          ? "Connect with any user name and the web interface password."
          : "Anyone on the network can open it. Set a password under System to ask for one."}
        It shows the folder under Save to.
      </p>
    </div>
  {/if}
</Card>
