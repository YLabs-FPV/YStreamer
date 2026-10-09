<script lang="ts">
  import { api, message } from "@/api/client";
  import { snackbar } from "@/stores/snackbar.svelte";
  import { settings } from "@/stores/settings.svelte";
  import { btn } from "@/components/ui/buttons";
  import type { Settings } from "@/types/settings";
  import type { StorageOption } from "@/types/recording";
  import { bytes, recording } from "@/stores/recording.svelte";
  import Card from "@/components/ui/Card.svelte";
  import FileList from "@/components/recording/FileList.svelte";
  import ButtonCard from "@/components/recording/ButtonCard.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";

  let {
    config = $bindable(),
    tab,
  }: { config: Settings["recording"]; tab: string } = $props();

  const FORMATS = [
    { label: "MP4", value: "mp4" },
    { label: "MPEG-TS", value: "ts" },
  ];

  const SPLITS = [
    { label: "One long file", value: 0 },
    ...[5, 10, 15, 30, 60].map((m) => ({
      label: `Every ${m} minutes`,
      value: m,
    })),
  ];

  const RESERVE = [
    { label: "250 MB", value: 250 },
    { label: "500 MB", value: 500 },
    { label: "1 GB", value: 1024 },
    { label: "2 GB", value: 2048 },
    { label: "5 GB", value: 5120 },
  ];

  let storageOptions = $state<StorageOption[]>([]);

  // "Somewhere else" keeps the free-text path field visible for custom folders
  const CUSTOM = "__custom__";
  let customPath = $state(false);
  const folderOptions = $derived([
    ...storageOptions.map((o) => ({
      label: o.missing
        ? `${o.label} (USB) - not plugged in`
        : `${o.label}${o.removable ? " (USB)" : ""} - ${bytes(o.free_bytes)} free`,
      value: o.path,
    })),
    { label: "Somewhere else…", value: CUSTOM },
  ]);
  const folderValue = $derived(
    customPath || !storageOptions.some((o) => o.path === config.path)
      ? CUSTOM
      : config.path,
  );

  const status = $derived(recording.status);
  const store = $derived(status?.storage);
  const usedPct = $derived(
    store && store.total_bytes
      ? ((store.total_bytes - store.free_bytes - store.used_bytes) /
          store.total_bytes) *
          100
      : 0,
  );
  const recPct = $derived(
    store && store.total_bytes
      ? (store.used_bytes / store.total_bytes) * 100
      : 0,
  );

  async function loadStorageOptions() {
    try {
      storageOptions = await api.storageOptions();
    } catch {
      // Not fatal: the path can still be typed in by hand
    }
  }

  // Only a drive that's there can be ejected
  const ejectable = $derived(
    storageOptions.find(
      (o) => o.path === config.path && o.removable && !o.missing,
    ),
  );
  let ejecting = $state(false);

  async function eject() {
    // The list refreshes below, and the drive leaves it
    const drive = ejectable;
    if (!drive) return;
    ejecting = true;
    try {
      await api.ejectDrive(drive.path);
      snackbar.show(`${drive.label} can be unplugged now`);
      await loadStorageOptions();
      recording.load();
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      ejecting = false;
    }
  }

  // The saved folder, not the draft: that's where the recorder writes
  const savedPath = $derived(settings.saved?.recording.path);

  // Again whenever the folder changes or the tab is opened, so the list is
  // never the previous folder's
  $effect(() => {
    void savedPath;
    void tab;
    recording.location = null;
    recording.load();
    recording.loadFiles();
    loadStorageOptions();
  });

  const location = $derived.by(() => {
    const option = storageOptions.find((o) => o.path === store?.path);
    if (!option) return store?.path ?? "";
    return option.removable ? `${option.label} (USB)` : option.label;
  });
</script>

<div class="flex flex-col gap-4">
  {#if tab === "files"}
    <Card title="Storage">
      {#snippet aside()}
        <span class="flex items-center gap-3 text-[12px]">
          <span class="max-w-48 truncate text-muted" title={store?.path}
            >{location}</span
          >
          <a href="/recording/settings" class="text-primary hover:underline"
            >Change</a
          >
        </span>
      {/snippet}
      {#if status?.error}
        <p class="bg-down/10 px-4 py-3 text-[13px] text-down">{status.error}</p>
      {/if}
      {#if store && !store.writable}
        <p class="bg-warn/10 px-4 py-3 text-[13px] text-fg">
          Can't write to <span class="font-mono">{store.path}</span>. Pick
          another folder under Settings.
        </p>
      {/if}
      {#if store}
        <div class="px-4 py-3">
          <div class="flex h-2 overflow-hidden rounded-full bg-surface-3">
            <div class="bg-line-strong" style="width: {usedPct}%"></div>
            <div class="bg-primary" style="width: {recPct}%"></div>
          </div>
          <div
            class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-muted"
          >
            <span class="flex items-center gap-1.5">
              <span class="h-1.5 w-1.5 rounded-full bg-primary"></span>
              Recordings {bytes(store.used_bytes)}
            </span>
            <span class="flex items-center gap-1.5">
              <span class="h-1.5 w-1.5 rounded-full bg-line-strong"></span>
              Other files
            </span>
            <span class="ml-auto font-mono tabular-nums">
              {bytes(store.free_bytes)} free of {bytes(store.total_bytes)}
            </span>
          </div>
        </div>
      {:else}
        <p class="px-4 py-3 text-[13px] text-muted">Loading…</p>
      {/if}
    </Card>

    <FileList
      places={storageOptions}
      savePath={store?.path ?? config.path}
      onchanged={() => {
        recording.load();
        loadStorageOptions();
      }}
    />
  {:else}
    <Card
      title="Settings"
      description="Applied to the next recording you start."
    >
      <Field label="Save to" for="rec-folder">
        <Select
          id="rec-folder"
          options={folderOptions}
          value={folderValue}
          onchange={(v) => {
            if (v === CUSTOM) {
              customPath = true;
            } else {
              customPath = false;
              config.path = String(v);
            }
          }}
        />
        {#if ejectable}
          <button
            type="button"
            class={btn.plain}
            disabled={ejecting}
            title="Finish writing so the drive can be unplugged"
            onclick={eject}
          >
            {ejecting ? "Ejecting…" : "Eject"}
          </button>
        {/if}
      </Field>
      {#if folderValue === CUSTOM}
        <Field label="Folder" for="rec-path">
          <TextField id="rec-path" mono bind:value={config.path} />
        </Field>
      {/if}

      <Field
        label="Format"
        help="Both survive a power cut mid-file. MP4 plays anywhere; MPEG-TS is slightly bigger."
        stacked
      >
        <div class="w-full sm:w-64">
          <Segmented
            options={FORMATS}
            value={config.format}
            onchange={(v) => (config.format = v)}
          />
        </div>
      </Field>

      <Field label="New file" for="rec-split">
        <Select
          id="rec-split"
          options={SPLITS}
          bind:value={config.split_minutes}
        />
      </Field>

      <Field label="File name prefix" for="rec-prefix">
        <TextField
          id="rec-prefix"
          mono
          bind:value={config.prefix}
          maxlength={32}
        />
      </Field>

      <Field label="Record automatically" for="rec-auto">
        <Switch id="rec-auto" bind:checked={config.auto_start} />
      </Field>

      <Field label="Keep free" for="rec-reserve">
        <Select
          id="rec-reserve"
          options={RESERVE}
          bind:value={config.reserve_mb}
        />
      </Field>
    </Card>

    <ButtonCard bind:button={config.button} />
  {/if}
</div>
