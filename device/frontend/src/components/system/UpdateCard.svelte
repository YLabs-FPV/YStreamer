<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import type { Release, UpdateStatus } from "@/types/system";
  import type { Settings } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import { update } from "@/stores/update.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import ReleaseNotes from "@/components/system/ReleaseNotes.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";

  let { advanced = $bindable() }: { advanced: Settings["advanced"] } = $props();

  const POLL_MS = 2000;

  let status = $state<UpdateStatus | null>(null);
  let busy = $state(false);
  let latest = $state<Release | null>(null);
  let checking = $state(false);
  let downloading = $state(false);
  /** The version being installed, from the request until it's running */
  let installing = $state<string | null>(null);
  /** Seen underway or restarting, so the same version reported again means done */
  let underway = false;
  let confirm = $state<"install" | "rollback" | null>(null);
  let confirmOpen = $state(false);
  let input: HTMLInputElement;

  onMount(() => {
    refresh();
    const timer = setInterval(() => installing && refresh(), POLL_MS);
    return () => clearInterval(timer);
  });

  async function refresh() {
    let next: UpdateStatus;
    try {
      next = await api.update();
    } catch {
      // Expected while the service restarts into the new version
      underway = true;
      return;
    }
    status = next;
    latest ??= next.available;
    update.available = next.available;
    if (!installing) return;
    if (next.installing) {
      underway = true;
    } else if (underway && next.current === installing && !next.failure) {
      // This page is the old version's; the new one comes with a reload
      location.reload();
    } else if (next.failure) {
      installing = null;
    }
  }

  async function upload(e: Event) {
    const files = [...((e.currentTarget as HTMLInputElement).files ?? [])];
    input.value = "";
    if (!files.length) return;
    const deb = files.find((f) => f.name.endsWith(".deb"));
    const sig = files.find((f) => f.name.endsWith(".sig"));
    if (!deb || !sig) {
      snackbar.show("Choose both the .deb file and its .sig file", true);
      return;
    }
    busy = true;
    try {
      status = await api.uploadUpdate(deb, (await sig.text()).trim());
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      busy = false;
    }
  }

  async function check() {
    checking = true;
    try {
      latest = await api.checkUpdate();
      update.available = latest.newer ? latest : null;
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      checking = false;
    }
  }

  async function download() {
    downloading = true;
    try {
      status = await api.downloadUpdate();
      latest = null;
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      downloading = false;
    }
  }

  async function discard() {
    try {
      status = await api.discardUpdate();
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  function ask(what: "install" | "rollback") {
    confirm = what;
    confirmOpen = true;
  }

  async function run() {
    const version = confirm === "install" ? status?.staged : status?.previous;
    if (!version) return;
    try {
      await (confirm === "install"
        ? api.installUpdate()
        : api.rollBackUpdate());
      underway = false;
      installing = version;
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }
</script>

<Card title="Updates">
  {#if !status}
    <p class="px-4 py-3 text-[13px] text-muted">Loading…</p>
  {:else if installing}
    <p class="px-4 py-3 text-[13px] text-muted">
      Installing {installing}… Video stops while YStreamer restarts, and this
      page reloads when it's done.
    </p>
  {:else}
    <Field label="Installed version">
      <span class="text-[13px] text-muted tabular-nums">{status.current}</span>
    </Field>
    {#if status.staged}
      <Field
        label="Ready to install"
        help={status.staged === status.current
          ? "This is the version already installed."
          : undefined}
      >
        <div class="flex items-center gap-2">
          <span class="text-[13px] text-fg tabular-nums">{status.staged}</span>
          <button type="button" class={btn.text} onclick={discard}>
            Discard
          </button>
          <button
            type="button"
            class={btn.primary}
            onclick={() => ask("install")}
          >
            Install
          </button>
        </div>
      </Field>
    {:else}
      {#if latest?.newer}
        <Field label="Version {latest.version} is available">
          <button
            type="button"
            class={btn.primary}
            disabled={downloading}
            onclick={download}
          >
            {downloading ? "Downloading…" : "Download"}
          </button>
        </Field>
        {#if latest.notes}
          <div class="px-4 pb-3">
            <ReleaseNotes notes={latest.notes} />
          </div>
        {/if}
      {:else}
        <Field
          label="Check for updates"
          help={latest ? "This is the latest version." : undefined}
        >
          <button
            type="button"
            class={btn.plain}
            disabled={checking}
            onclick={check}
          >
            {checking ? "Checking…" : "Check"}
          </button>
        </Field>
      {/if}
      <Field
        label="Install from files"
        help="Choose a release's .deb file together with its .sig file."
      >
        <button
          type="button"
          class={btn.plain}
          disabled={busy}
          onclick={() => input.click()}
        >
          {busy ? "Checking…" : "Choose files…"}
        </button>
      </Field>
    {/if}
    {#if status.previous}
      <Field label="Previous version">
        <div class="flex items-center gap-2">
          <span class="text-[13px] text-muted tabular-nums">
            {status.previous}
          </span>
          <button
            type="button"
            class={btn.plain}
            onclick={() => ask("rollback")}
          >
            Roll back
          </button>
        </div>
      </Field>
    {/if}
    <Field
      label="Check automatically"
      for="update-check"
      help="Asks the update server about twice a day. Nothing is downloaded or installed without you."
    >
      <Switch id="update-check" bind:checked={advanced.check_for_updates} />
    </Field>
    {#if status.failure}
      <div class="border-t border-line px-4 py-3">
        <p class="text-[13px] text-down">
          {#if status.failure.reverted}
            Version {status.failure.version} didn't start, so the device went back
            to {status.current}.
          {:else}
            Installing {status.failure.version} failed.
          {/if}
        </p>
        {#if status.failure.log}
          <pre
            class="mt-2 max-h-48 overflow-auto rounded-lg bg-surface-2 p-3 font-mono text-[11px] whitespace-pre-wrap text-muted">{status
              .failure.log}</pre>
        {/if}
      </div>
    {/if}
  {/if}
  <input
    bind:this={input}
    type="file"
    accept=".deb,.sig"
    multiple
    class="hidden"
    onchange={upload}
  />
</Card>

<ConfirmDialog
  bind:open={confirmOpen}
  title={confirm === "install"
    ? `Install ${status?.staged}?`
    : `Roll back to ${status?.previous}?`}
  message="Video, streams and any recording stop while YStreamer restarts. Don't unplug the device until it's back."
  confirmLabel={confirm === "install" ? "Install" : "Roll back"}
  onconfirm={run}
/>
