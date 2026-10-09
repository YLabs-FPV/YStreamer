<script lang="ts">
  import { settings } from "@/stores/settings.svelte";
  import { snackbar } from "@/stores/snackbar.svelte";
  import NetworkPage from "@/pages/NetworkPage.svelte";
  import InputPage from "@/pages/InputPage.svelte";
  import StreamingPage from "@/pages/StreamingPage.svelte";
  import SystemPage from "@/pages/SystemPage.svelte";
  import DisplayPage from "@/pages/DisplayPage.svelte";
  import RecordingPage from "@/pages/RecordingPage.svelte";
  import PerformancePage from "@/pages/PerformancePage.svelte";
  import LogsPage from "@/pages/LogsPage.svelte";
  import { developer } from "@/stores/developer.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";
  import Tabs from "@/components/ui/Tabs.svelte";
  import type { NavPage } from "@/lib/nav";
  let { page, tab: wanted }: { page: NavPage; tab: string } = $props();

  const tabs = $derived(
    (page.tabs ?? []).filter((t) => !t.developer || developer.enabled),
  );
  const tab = $derived(
    (tabs.find((t) => t.id === wanted) ?? tabs[0])?.id ?? "",
  );

  let confirmWifi = $state(false);

  function requestSave() {
    const wifi = settings.dirty.includes("wifi");
    if (wifi && settings.saved?.wifi.mode !== settings.draft?.wifi.mode) {
      confirmWifi = true;
    } else {
      save();
    }
  }

  async function save() {
    const notices = await settings.save();
    if (settings.error) return;
    snackbar.show(notices.join(" ") || "Settings saved");
  }
</script>

<div class="pb-24">
  {#if tabs.length}
    <Tabs {page} {tabs} current={tab} />
  {/if}
  {#if settings.loadError}
    <div
      class="rounded-xl border border-line bg-surface px-4 py-6 text-center shadow-sm"
    >
      <p class="text-[13px] text-fg">Couldn't load settings</p>
      <p class="mt-1 text-[12px] text-muted">{settings.loadError}</p>
      <button
        type="button"
        class="{btn.plain} mt-4"
        onclick={() => settings.load()}>Try again</button
      >
    </div>
  {:else if !settings.draft || !settings.saved}
    <p class="py-6 text-[13px] text-muted">Loading settings…</p>
  {:else if page.id === "input"}
    <InputPage bind:input={settings.draft.input} />
  {:else if page.id === "network"}
    <NetworkPage
      bind:wifi={settings.draft.wifi}
      bind:ethernet={settings.draft.ethernet}
      {tab}
    />
  {:else if page.id === "streaming"}
    <StreamingPage
      bind:rtsp={settings.draft.rtsp}
      bind:srt={settings.draft.srt}
      bind:rtmp={settings.draft.rtmp}
      bind:udp={settings.draft.udp}
      bind:viewer={settings.draft.viewer}
      saved={settings.saved.rtsp}
      savedSrt={settings.saved.srt}
      savedRtmp={settings.saved.rtmp}
      savedUdp={settings.saved.udp}
      {tab}
    />
  {:else if page.id === "recording"}
    <RecordingPage bind:config={settings.draft.recording} {tab} />
  {:else if page.id === "terminal"}
    {#if developer.enabled}
      <!-- Loaded on demand: the terminal emulator is a third of the bundle -->
      {#await import("@/pages/TerminalPage.svelte") then { default: TerminalPage }}
        <TerminalPage />
      {/await}
    {:else}
      <p class="text-[13px] text-muted">
        The terminal is a developer tool. Turn on developer mode first.
      </p>
    {/if}
  {:else if page.id === "performance"}
    <PerformancePage />
  {:else if page.id === "logs"}
    <LogsPage />
  {:else if page.id === "display"}
    <DisplayPage
      bind:display={settings.draft.display}
      bind:splash={settings.draft.splash}
      bind:overlay={settings.draft.overlay}
      {tab}
    />
  {:else}
    <SystemPage
      bind:device={settings.draft.device}
      bind:advanced={settings.draft.advanced}
      {tab}
    />
  {/if}
</div>

{#if settings.dirty.length}
  <div
    class="fixed right-0 bottom-0 left-0 z-20 border-t border-line bg-surface shadow-[0_-2px_8px_rgb(0_0_0/0.06)] md:left-56"
  >
    <div
      class="mx-auto flex w-full max-w-4xl items-center gap-3 px-4 py-3 sm:px-8"
    >
      <p
        class="min-w-0 flex-1 truncate text-[13px] {settings.error
          ? 'text-down'
          : 'text-muted'}"
      >
        {settings.error ?? "You have unsaved changes"}
      </p>
      <button
        type="button"
        class={btn.text}
        disabled={settings.saving}
        onclick={() => settings.discard()}
      >
        Discard
      </button>
      <button
        type="button"
        class={btn.primary}
        disabled={settings.saving}
        onclick={requestSave}
      >
        {settings.saving ? "Saving…" : "Save"}
      </button>
    </div>
  </div>
{/if}

<ConfirmDialog
  bind:open={confirmWifi}
  title="Change WiFi mode?"
  message="If you're connected to this page over WiFi, you'll lose the connection. Reconnect using the new settings."
  confirmLabel="Save and apply"
  onconfirm={save}
/>
