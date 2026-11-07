<script lang="ts">
  import { api, message } from "@/api/client";
  import { control } from "@/stores/control.svelte";
  import HoldButton from "@/components/ui/HoldButton.svelte";
  import type { AircraftAction } from "@/types/aircraft";

  const LABEL: Record<AircraftAction, string> = {
    takeoff: "Take off",
    land: "Land",
    return_home: "Return to home",
    cancel_return_home: "Cancel RTH",
    cancel_landing: "Cancel landing",
  };

  let busy = $state(false);
  let last = $state<{ text: string; tone: string } | null>(null);

  const waiting = $derived(
    control.link !== "live"
      ? "Waiting for the goggles link"
      : !control.aircraft
        ? "Waiting for the aircraft"
        : null,
  );

  async function send(action: AircraftAction) {
    busy = true;
    const at = new Date().toLocaleTimeString();
    const what = LABEL[action];
    try {
      const r = await api.aircraftAction(action);
      last = r.acknowledged
        ? { text: `${at} · ${what}: the aircraft answered`, tone: "text-ok" }
        : {
            text: `${at} · ${what}: sent ${r.sends}×, no answer from the aircraft. Watch it and be ready on the remote.`,
            tone: "text-warn",
          };
    } catch (e) {
      last = { text: `${at} · ${what}: ${message(e)}`, tone: "text-down" };
    } finally {
      busy = false;
    }
  }
</script>

<section
  class="overflow-hidden rounded-xl border border-line bg-surface shadow-sm"
>
  <header
    class="flex items-center justify-between gap-3 border-b border-line bg-surface-2/40 px-4 py-3"
  >
    <h2 class="truncate text-[13px] font-medium text-fg">
      {control.aircraft?.name ?? "Aircraft"}
      <span class="font-normal text-muted">· actions</span>
    </h2>
    <span class="shrink-0 text-[12px] text-muted">Hold to send</span>
  </header>

  <p class="bg-warn/10 px-4 py-2.5 text-[12px] text-fg">
    You're still the pilot. Keep the remote in hand, ready to take over.
  </p>

  <div class="grid grid-cols-2 gap-2 px-4 py-3">
    {#each ["takeoff", "land", "return_home", "cancel_return_home"] as const as action}
      <HoldButton
        label={LABEL[action]}
        disabled={busy || waiting !== null}
        onconfirm={() => send(action)}
      />
    {/each}
    <div class="col-span-2 grid">
      <HoldButton
        label={LABEL.cancel_landing}
        disabled={busy || waiting !== null}
        onconfirm={() => send("cancel_landing")}
      />
    </div>
  </div>

  {#if waiting || busy || last}
    <p
      class="border-t border-line px-4 py-2.5 text-[12px] {waiting || busy
        ? 'text-muted'
        : last?.tone}"
    >
      {waiting ?? (busy ? "Sending…" : last?.text)}
    </p>
  {/if}
</section>
