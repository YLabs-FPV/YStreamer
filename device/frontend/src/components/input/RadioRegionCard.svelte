<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, message } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import type { RadioRegion, RadioRegionStatus } from "@/types/input";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";
  import { snackbar } from "@/stores/snackbar.svelte";

  let { region = $bindable() }: { region: Settings["radio_region"] } = $props();

  const REGIONS: { label: string; value: RadioRegion }[] = [
    { label: "FCC", value: "us" },
    { label: "CE", value: "eu" },
    { label: "Indonesia", value: "id" },
  ];

  const ABOUT: Record<RadioRegion, string> = {
    us: "FCC limits, as in the United States.",
    eu: "CE limits, as in Europe. The radios' factory setting.",
    id: "Indonesia's limits.",
  };

  const NAME: Record<RadioRegion, string> = {
    us: "FCC",
    eu: "CE",
    id: "Indonesia",
  };

  let status = $state<RadioRegionStatus | null>(null);
  let confirming = $state(false);
  let sending = $state(false);
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh() {
    try {
      status = await api.radioRegion();
    } catch {
      // Shown as offline elsewhere
    }
  }

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 2000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  // Turning it on asks first; turning it off doesn't
  function toggle(on: boolean) {
    if (on) confirming = true;
    else region.enabled = false;
  }

  async function sendAgain() {
    sending = true;
    try {
      await api.applyRadioRegion();
      snackbar.show("Sending the region again");
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      sending = false;
    }
  }

  const summary = $derived.by(() => {
    if (!status) return null;
    if (status.enabled && status.active) {
      const secs = Math.round((status.sent_ms_ago ?? 0) / 1000);
      return {
        text: `${NAME[status.active]} sent to the radios ${secs} s ago.`,
        tone: "text-ok",
      };
    }
    if (status.enabled) {
      return {
        text: "Waiting for the goggles. The region goes out once they're connected.",
        tone: "text-muted",
      };
    }
    if (status.applied) {
      return {
        text: "CE goes back to the radios when the goggles and aircraft are next connected.",
        tone: "text-warn",
      };
    }
    return null;
  });
</script>

<Card
  title="Radio region"
  description="The transmit limits the goggles and aircraft follow, the region DJI's own apps set by country."
>
  {#snippet aside()}
    {#if status?.enabled}
      <button
        type="button"
        class={btn.plain}
        disabled={sending}
        onclick={sendAgain}
      >
        Send again
      </button>
    {/if}
  {/snippet}

  <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
    Transmit-power limits differ by country. You're responsible for following
    your local radio rules.
  </p>
  {#if summary}
    <p class="px-4 py-3 text-[12px] {summary.tone}">{summary.text}</p>
  {/if}

  <Field
    label="Set the radio region"
    for="radio-region"
    help="Off leaves the radios as DJI's apps last set them. Turning it off after this unit set a region puts CE back."
  >
    <Switch
      id="radio-region"
      bind:checked={() => region.enabled, (on) => toggle(on)}
    />
  </Field>

  {#if region.enabled}
    <Field label="Region" help={ABOUT[region.region]} stacked>
      <div class="w-full sm:w-80">
        <Segmented
          options={REGIONS}
          value={region.region}
          onchange={(v) => (region.region = v)}
        />
      </div>
    </Field>
  {/if}
</Card>

<ConfirmDialog
  bind:open={confirming}
  title="Set the radio region?"
  message="Transmit-power limits differ by country. You're responsible for following your local radio rules in the region you pick."
  confirmLabel="Turn on"
  onconfirm={() => (region.enabled = true)}
/>
