<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { api, message } from "@/api/client";
  import { bytes, recording } from "@/stores/recording.svelte";
  import { transfer } from "@/stores/transfer.svelte";
  import { snackbar } from "@/stores/snackbar.svelte";
  import type { RecordingFile, StorageOption } from "@/types/recording";
  import Card from "@/components/ui/Card.svelte";
  import Select from "@/components/ui/Select.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";
  import FileVideo from "@lucide/svelte/icons/file-video";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import Download from "@lucide/svelte/icons/download";
  import Play from "@lucide/svelte/icons/play";
  import LayoutGrid from "@lucide/svelte/icons/layout-grid";
  import List from "@lucide/svelte/icons/list";

  let {
    places,
    savePath,
    onchanged,
  }: {
    /** Everywhere recordings can be: the card and plugged-in drives */
    places: StorageOption[];
    /** Where new recordings are saved */
    savePath: string;
    /** Something was copied, moved or ejected, so free space is stale */
    onchanged: () => void;
  } = $props();

  type View = "grid" | "list";
  const VIEW_KEY = "ystreamer.files.view";
  const STEP: Record<View, number> = { grid: 12, list: 30 };

  function restoreView(): View {
    try {
      return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "grid";
    } catch {
      return "grid";
    }
  }

  let view = $state<View>(restoreView());
  let shown = $state(STEP[restoreView()]);
  let sentinel = $state<HTMLDivElement>();

  function setView(v: View) {
    view = v;
    shown = STEP[v];
    try {
      localStorage.setItem(VIEW_KEY, v);
    } catch {}
  }

  const visible = $derived(recording.files.slice(0, shown));

  $effect(() => {
    void shown;
    if (!sentinel) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) shown += STEP[view];
      },
      { rootMargin: "400px" },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  });

  const here = $derived(recording.location ?? savePath);
  const present = $derived(places.filter((p) => !p.missing));
  const elsewhere = $derived(present.filter((p) => p.path !== here));
  const nameOf = (path: string) => {
    const place = places.find((p) => p.path === path);
    if (!place) return path;
    return place.removable ? `${place.label} (USB)` : place.label;
  };

  function look(path: string) {
    recording.location = path === savePath ? null : path;
    selected.clear();
    shown = STEP[view];
    recording.loadFiles();
  }

  let dest = $state("");
  $effect(() => {
    if (!elsewhere.some((p) => p.path === dest))
      dest = elsewhere[0]?.path ?? "";
  });

  const selected = new SvelteSet<string>();
  function toggle(name: string) {
    if (selected.has(name)) selected.delete(name);
    else selected.add(name);
  }
  // Names that are gone can't stay picked
  $effect(() => {
    const names = new Set(recording.files.map((f) => f.name));
    for (const name of selected) if (!names.has(name)) selected.delete(name);
  });

  function send(files: string[] | undefined, move: boolean) {
    transfer.start({ from: here, to: dest, files, move });
    selected.clear();
  }

  let confirmMove = $state(false);

  $effect(() => {
    transfer.onfinish = () => {
      recording.loadFiles();
      recording.load();
      onchanged();
    };
    return () => (transfer.onfinish = null);
  });

  const job = $derived(transfer.progress);
  // Moves within one drive copy nothing, so they go by files instead
  const jobPct = $derived.by(() => {
    if (!job) return 0;
    if (job.state !== "running") return 100;
    if (job.bytes_total) return (job.bytes_done / job.bytes_total) * 100;
    return job.files_total ? (job.files_done / job.files_total) * 100 : 0;
  });
  const count = (n: number) => `${n} recording${n === 1 ? "" : "s"}`;
  const jobSummary = $derived.by(() => {
    if (!job) return "";
    const copied = job.files_done;
    const verb = job.moving ? "Moved" : "Copied";
    const already = job.skipped
      ? ` ${job.skipped} ${job.skipped === 1 ? "was" : "were"} already there.`
      : "";
    if (job.state === "done")
      return job.files_done === 0 && job.skipped
        ? `Everything is already on ${nameOf(job.to)}.`
        : `${verb} ${count(copied)} to ${nameOf(job.to)}.${job.moving ? "" : already}`;
    if (job.state === "cancelled")
      return `Stopped after ${count(job.files_done)}. The rest is untouched.`;
    return job.error ?? "The transfer failed.";
  });

  // A drive that just received recordings is usually about to be unplugged
  const ejectable = $derived(
    job?.state === "done"
      ? present.find((p) => p.path === job.to && p.removable)
      : undefined,
  );
  let ejecting = $state(false);
  async function eject() {
    // Clearing the finished transfer below also clears `ejectable`
    const drive = ejectable;
    if (!drive) return;
    ejecting = true;
    try {
      await api.ejectDrive(drive.path);
      snackbar.show(`${drive.label} can be unplugged now`);
      await transfer.cancel();
      if (recording.location === drive.path) look(savePath);
      onchanged();
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      ejecting = false;
    }
  }

  let preview = $state<string | null>(null);
  let pendingDelete = $state<string | null>(null);
  let confirmDelete = $state(false);

  function askDelete(name: string) {
    pendingDelete = name;
    confirmDelete = true;
  }

  const playable = (f: RecordingFile) => f.name.endsWith(".mp4");
  const beingWritten = (f: RecordingFile) =>
    recording.location === null &&
    recording.active &&
    !!recording.status?.file?.endsWith(f.name);
  const url = (name: string) => api.recordingUrl(name, recording.location);

  const when = (unix: number) =>
    new Date(unix * 1000).toLocaleString(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    });

  const iconBtn =
    "grid h-8 w-8 place-items-center rounded-md text-faint transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary";
</script>

<Card title="Recordings">
  {#snippet aside()}
    <div class="flex items-center gap-2">
      {#if present.length > 1}
        <Select
          class="sm:w-44"
          value={here}
          options={present.map((p) => ({
            label: nameOf(p.path),
            value: p.path,
          }))}
          onchange={(v) => look(String(v))}
        />
      {/if}
      <div class="flex rounded-md border border-line bg-surface-2 p-0.5">
        {@render viewButton("grid", "Grid view", LayoutGrid)}
        {@render viewButton("list", "List view", List)}
      </div>
      <button
        type="button"
        class={btn.plain}
        disabled={recording.loadingFiles}
        onclick={() => recording.loadFiles()}
      >
        {recording.loadingFiles ? "Loading…" : "Refresh"}
      </button>
    </div>
  {/snippet}

  {#if job}
    <div class="border-b border-line px-4 py-3">
      {#if job.state === "running"}
        <div class="flex items-center gap-3">
          <div class="min-w-0 flex-1">
            <p class="text-[13px] text-fg">
              {job.moving ? "Moving" : "Copying"}
              {job.files_total === 1
                ? "1 recording"
                : `recording ${Math.min(job.files_done + 1, job.files_total)} of ${job.files_total}`}
              to {nameOf(job.to)}
              <span class="text-muted">
                · {bytes(job.bytes_done)} of {bytes(job.bytes_total)}
              </span>
            </p>
            <p class="truncate font-mono text-[11px] text-muted">
              {job.current ?? ""}
            </p>
          </div>
          <button
            type="button"
            class={btn.plain}
            onclick={() => transfer.cancel()}>Cancel</button
          >
        </div>
        <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-surface-3">
          <div
            class="h-full bg-primary transition-[width] duration-500"
            style="width: {jobPct}%"
          ></div>
        </div>
      {:else}
        <div class="flex flex-wrap items-center gap-3">
          <p
            class="min-w-0 flex-1 text-[13px] {job.state === 'failed'
              ? 'text-down'
              : 'text-fg'}"
          >
            {jobSummary}
          </p>
          {#if ejectable}
            <button
              type="button"
              class={btn.plain}
              disabled={ejecting}
              onclick={eject}
            >
              {ejecting ? "Ejecting…" : `Eject ${ejectable.label}`}
            </button>
          {/if}
          <button
            type="button"
            class={btn.text}
            onclick={() => transfer.cancel()}>Dismiss</button
          >
        </div>
      {/if}
    </div>
  {/if}

  {#if elsewhere.length > 0 && recording.files.length > 0}
    <div
      class="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2.5"
    >
      <span class="text-[12px] text-muted">To</span>
      <Select
        class="sm:w-44"
        bind:value={dest}
        options={elsewhere.map((p) => ({
          label: nameOf(p.path),
          value: p.path,
        }))}
      />
      <button
        type="button"
        class={btn.plain}
        disabled={selected.size === 0 || transfer.running}
        onclick={() => send([...selected], false)}
      >
        Copy{selected.size ? ` ${selected.size} selected` : " selected"}
      </button>
      <button
        type="button"
        class={btn.plain}
        disabled={selected.size === 0 || transfer.running}
        onclick={() => (confirmMove = true)}
      >
        Move{selected.size ? ` ${selected.size} selected` : " selected"}
      </button>
      <button
        type="button"
        class="{btn.plain} sm:ml-auto"
        disabled={transfer.running}
        title="Recordings that aren't on {nameOf(dest)} yet"
        onclick={() => send(undefined, false)}
      >
        Copy everything new
      </button>
    </div>
  {/if}

  {#if recording.files.length === 0}
    <p class="px-4 py-6 text-center text-[13px] text-muted">
      {recording.loadingFiles ? "Loading…" : "No recordings yet."}
    </p>
  {:else if view === "grid"}
    <div class="grid grid-cols-2 gap-3 p-4 sm:grid-cols-3">
      {#each visible as f (f.name)}
        <div class="min-w-0">
          <div
            class="group relative aspect-video overflow-hidden rounded-lg border border-line bg-black"
          >
            {#if playable(f) && !beingWritten(f)}
              <video
                src="{url(f.name)}#t=0.5"
                preload="metadata"
                muted
                playsinline
                tabindex="-1"
                class="pointer-events-none h-full w-full object-cover"
              ></video>
              <button
                type="button"
                onclick={() => (preview = f.name)}
                aria-label="Play {f.name}"
                class="absolute inset-0 grid place-items-center bg-black/0 transition-colors group-hover:bg-black/30 focus-visible:bg-black/30 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary"
              >
                <span
                  class="grid h-10 w-10 place-items-center rounded-full bg-black/60 text-white opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100"
                >
                  <Play class="h-4 w-4 fill-current" aria-hidden="true" />
                </span>
              </button>
            {:else}
              <div class="grid h-full w-full place-items-center bg-surface-2">
                <FileVideo class="h-6 w-6 text-faint" aria-hidden="true" />
              </div>
            {/if}
            {#if beingWritten(f)}
              {@render recordingBadge("absolute top-2 left-2")}
            {:else if elsewhere.length > 0}
              <input
                type="checkbox"
                checked={selected.has(f.name)}
                onchange={() => toggle(f.name)}
                aria-label="Select {f.name}"
                class="absolute top-2 right-2 h-4 w-4 accent-primary"
              />
            {/if}
          </div>
          <div class="mt-1.5 flex items-start gap-1">
            <div class="min-w-0 flex-1">
              <p class="truncate text-[12px] text-fg">{when(f.modified)}</p>
              <p class="truncate text-[11px] text-muted" title={f.name}>
                {bytes(f.size_bytes)} · {f.name}
              </p>
            </div>
            {@render fileActions(f)}
          </div>
        </div>
      {/each}
    </div>
  {:else}
    {#each visible as f (f.name)}
      <div class="flex items-center gap-3 px-4 py-2.5">
        {#if elsewhere.length > 0}
          <input
            type="checkbox"
            checked={selected.has(f.name)}
            disabled={beingWritten(f)}
            onchange={() => toggle(f.name)}
            aria-label="Select {f.name}"
            class="h-4 w-4 shrink-0 accent-primary"
          />
        {/if}
        <FileVideo class="h-4 w-4 shrink-0 text-faint" aria-hidden="true" />
        <div class="min-w-0 flex-1">
          <p class="flex items-center gap-2">
            <span class="truncate font-mono text-[12px] text-fg">{f.name}</span>
            {#if beingWritten(f)}
              {@render recordingBadge("shrink-0")}
            {/if}
          </p>
          <p class="text-[11px] text-muted">
            {when(f.modified)} · {bytes(f.size_bytes)}
          </p>
        </div>
        <div class="flex shrink-0 items-center">
          {#if playable(f) && !beingWritten(f)}
            <button
              type="button"
              class="{iconBtn} hover:bg-surface-2 hover:text-fg"
              onclick={() => (preview = f.name)}
              aria-label="Play {f.name}"
              title="Play"
            >
              <Play class="h-4 w-4" aria-hidden="true" />
            </button>
          {/if}
          {@render fileActions(f)}
        </div>
      </div>
    {/each}
  {/if}

  {#if shown < recording.files.length}
    <div
      bind:this={sentinel}
      class="px-4 py-3 text-center text-[12px] text-muted"
    >
      Showing {shown} of {recording.files.length}
    </div>
  {/if}
</Card>

{#snippet viewButton(v: View, label: string, Icon: typeof List)}
  <button
    type="button"
    onclick={() => setView(v)}
    aria-label={label}
    aria-pressed={view === v}
    title={label}
    class="grid h-7 w-7 place-items-center rounded transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary
      {view === v
      ? 'bg-surface text-fg shadow-sm'
      : 'text-faint hover:text-fg'}"
  >
    <Icon class="h-3.5 w-3.5" aria-hidden="true" />
  </button>
{/snippet}

{#snippet recordingBadge(extra: string)}
  <span
    class="flex items-center gap-1.5 rounded-full bg-down px-2 py-0.5 text-[10px] font-semibold tracking-wide text-white uppercase {extra}"
  >
    <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-white"></span>
    Recording
  </span>
{/snippet}

{#snippet fileActions(f: RecordingFile)}
  <a
    class="{iconBtn} shrink-0 hover:bg-surface-2 hover:text-fg"
    href={url(f.name)}
    download={f.name}
    aria-label="Download {f.name}"
    title="Download"
  >
    <Download class="h-4 w-4" aria-hidden="true" />
  </a>
  <button
    type="button"
    class="{iconBtn} shrink-0 hover:bg-down hover:text-white"
    onclick={() => askDelete(f.name)}
    aria-label="Delete {f.name}"
    title="Delete"
  >
    <Trash2 class="h-4 w-4" aria-hidden="true" />
  </button>
{/snippet}

{#if preview}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="fixed inset-0 z-40 grid place-items-center bg-black/70 p-4"
    onclick={(e) => e.target === e.currentTarget && (preview = null)}
  >
    <div
      class="w-full max-w-3xl overflow-hidden rounded-xl border border-line bg-surface shadow-lg"
    >
      <div class="flex items-center gap-3 border-b border-line px-4 py-2.5">
        <span class="min-w-0 flex-1 truncate font-mono text-[12px] text-fg"
          >{preview}</span
        >
        <button type="button" class={btn.text} onclick={() => (preview = null)}
          >Close</button
        >
      </div>
      <video src={url(preview)} controls autoplay class="w-full bg-black"
      ></video>
    </div>
  </div>
{/if}

<ConfirmDialog
  bind:open={confirmMove}
  title="Move {count(selected.size)} to {nameOf(dest)}?"
  message="Each one is removed from {nameOf(
    here,
  )} once its copy is complete. Nothing is removed if the copy fails."
  confirmLabel="Move"
  onconfirm={() => send([...selected], true)}
/>

<ConfirmDialog
  bind:open={confirmDelete}
  title="Delete this recording?"
  message="{pendingDelete} will be removed from {nameOf(
    here,
  )}. This can't be undone."
  confirmLabel="Delete"
  danger
  onconfirm={() => pendingDelete && recording.remove(pendingDelete)}
/>
