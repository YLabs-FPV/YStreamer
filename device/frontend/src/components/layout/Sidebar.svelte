<script lang="ts">
  import { control } from "@/stores/control.svelte";
  import { settings } from "@/stores/settings.svelte";
  import { recording } from "@/stores/recording.svelte";
  import { WATCH, NAV_GROUPS, type NavPage } from "@/lib/nav";
  import { auth } from "@/stores/auth.svelte";
  import { developer } from "@/stores/developer.svelte";
  import { message } from "@/api/client";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Battery from "@/components/ui/Battery.svelte";
  import SignalBars from "@/components/ui/SignalBars.svelte";
  import Brand from "@/components/ui/Brand.svelte";
  import Lock from "@lucide/svelte/icons/lock";
  import LogOut from "@lucide/svelte/icons/log-out";
  import LogIn from "@lucide/svelte/icons/log-in";
  import CircleArrowUp from "@lucide/svelte/icons/circle-arrow-up";
  import { update } from "@/stores/update.svelte";
  import { transfer } from "@/stores/transfer.svelte";
  import ArrowRightLeft from "@lucide/svelte/icons/arrow-right-left";

  const terminalUsable = $derived(
    developer.enabled &&
      auth.admin &&
      auth.enabled &&
      !!settings.saved?.advanced.terminal,
  );

  let {
    current,
    onnavigate,
  }: { current: string; onnavigate: (id: string) => void } = $props();

  const uvcStatus = $derived.by(() => {
    const u = control.uvc;
    if (u?.state === "live")
      return { text: u.device ?? "USB camera", dot: "bg-ok" };
    if (u?.state === "starting")
      return { text: `Starting ${u.device ?? "camera"}`, dot: "bg-warn" };
    if (u?.state === "error") return { text: "Camera error", dot: "bg-down" };
    return { text: "No USB camera", dot: "bg-faint" };
  });

  const legacyStatus = $derived.by(() => {
    switch (control.legacy?.state) {
      case "live":
        return { text: "FPV Goggles", dot: "bg-ok" };
      case "waiting":
        return { text: "FPV Goggles, no video", dot: "bg-warn" };
      case "error":
        return { text: "Goggles error", dot: "bg-down" };
      default:
        return { text: "No goggles", dot: "bg-faint" };
    }
  });

  const status = $derived(
    !control.connected
      ? { text: "Offline", dot: "bg-down" }
      : control.inputMode === "uvc"
        ? uvcStatus
        : control.inputMode === "dji_fpv_legacy"
          ? legacyStatus
          : control.link === "live"
            ? {
                text: control.goggles?.name ?? "Goggles connected",
                dot: "bg-ok",
              }
            : control.link === "connecting"
              ? { text: "Connecting to goggles", dot: "bg-warn" }
              : { text: "No goggles", dot: "bg-faint" },
  );

  $effect(() => {
    if (auth.admin) {
      update.refresh();
      transfer.check();
    }
  });

  async function logout() {
    try {
      await auth.logout();
      snackbar.show("Logged out");
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  const isDirty = (p: NavPage) =>
    p.sections.some((s) => settings.dirty.includes(s));
</script>

<div class="flex h-full flex-col">
  <div class="px-5 pt-6 pb-5">
    <Brand />
  </div>

  <nav
    aria-label="Main"
    class="flex flex-1 flex-col gap-0.5 overflow-y-auto px-3"
  >
    {@render item(WATCH)}

    {#each NAV_GROUPS as group}
      <span
        class="mt-5 mb-1.5 px-3 text-[11px] tracking-wide text-faint uppercase"
        >{group.label}</span
      >
      {#each group.pages.filter((p) => p.id !== "terminal" || terminalUsable) as p}
        {@render item(p)}
      {/each}
    {/each}
  </nav>

  {#if auth.admin && update.available}
    <div class="px-3 pb-2">
      <a
        href="/system/updates"
        onclick={() => onnavigate("system")}
        class="flex w-full items-center gap-3 rounded-md bg-primary/10 px-3 py-2 text-[13px] font-medium text-primary transition-colors hover:bg-primary/20 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary"
      >
        <CircleArrowUp class="h-4 w-4 shrink-0" aria-hidden="true" />
        <span class="truncate"
          >Version {update.available.version} available</span
        >
      </a>
    </div>
  {/if}

  {#if auth.enabled && auth.admin}
    <div class="px-3 pb-2">
      <button
        type="button"
        onclick={logout}
        class="flex w-full items-center gap-3 rounded-md px-3 py-2 text-[13px] font-medium text-muted transition-colors hover:bg-surface-2/60 hover:text-fg focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary"
      >
        <LogOut class="h-4 w-4 shrink-0" aria-hidden="true" />
        Log out
      </button>
    </div>
  {:else if auth.enabled}
    <div class="px-3 pb-2">
      <a
        href="/login"
        onclick={() => onnavigate("login")}
        aria-current={current === "login" ? "page" : undefined}
        class="flex w-full items-center gap-3 rounded-md px-3 py-2 text-[13px] font-medium transition-colors focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary
          {current === 'login'
          ? 'bg-surface-2 text-fg'
          : 'text-muted hover:bg-surface-2/60 hover:text-fg'}"
      >
        <LogIn class="h-4 w-4 shrink-0" aria-hidden="true" />
        Log in
      </a>
    </div>
  {/if}

  <div
    class="flex items-center justify-between gap-3 border-t border-line px-5 py-4"
  >
    <div class="flex min-w-0 flex-col gap-0.5">
      <span class="flex items-center gap-2 text-[12px] text-muted">
        <span class="h-1.5 w-1.5 shrink-0 rounded-full {status.dot}"></span>
        <span class="truncate" title={control.goggles?.code}>{status.text}</span
        >
      </span>
      {#if control.inputMode === "dji_fpv" && control.link === "live"}
        <span
          class="truncate pl-3.5 text-[11px] {control.aircraft
            ? 'text-muted'
            : 'text-faint'}"
          title={control.aircraft?.code}
        >
          {control.aircraft?.name ?? "No aircraft linked"}
        </span>
      {/if}
    </div>
    <div class="flex shrink-0 flex-col items-end gap-1.5">
      <SignalBars quality={control.aircraft ? control.linkQuality : null} />
      <Battery percent={control.gogglesBattery} />
    </div>
  </div>
</div>

{#snippet item(p: NavPage)}
  {@const active = p.id === current}
  <a
    href="/{p.id === 'watch' ? '' : p.id}"
    onclick={() => onnavigate(p.id)}
    aria-current={active ? "page" : undefined}
    class="flex items-center gap-3 rounded-md px-3 py-2 text-[13px] font-medium transition-colors focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary
      {active
      ? 'bg-surface-2 text-fg'
      : 'text-muted hover:bg-surface-2/60 hover:text-fg'}"
  >
    <p.icon
      class="h-4 w-4 shrink-0 {active ? 'text-primary' : ''}"
      aria-hidden="true"
    />
    <span class="flex-1">{p.label}</span>
    {#if p.id === "recording" && transfer.running}
      <span title="Copying recordings">
        <ArrowRightLeft
          class="h-3.5 w-3.5 animate-pulse text-primary"
          aria-label="Copying recordings"
        />
      </span>
    {/if}
    {#if p.id === "recording" && recording.active}
      <span class="h-2 w-2 animate-pulse rounded-full bg-down" title="Recording"
      ></span>
    {/if}
    {#if isDirty(p)}
      <span class="h-1.5 w-1.5 rounded-full bg-accent" title="Unsaved changes"
      ></span>
    {/if}
    {#if p.id !== "watch" && !auth.admin}
      <Lock class="h-3.5 w-3.5 text-faint" aria-label="Needs a login" />
    {/if}
  </a>
{/snippet}
