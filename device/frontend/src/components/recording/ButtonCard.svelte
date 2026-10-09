<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import type { ButtonStatus } from "@/types/recording";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";

  let { button = $bindable() }: { button: Settings["recording"]["button"] } =
    $props();

  // GPIO21 (pin 40) is left out: grounding it while starting resets the settings
  // prettier-ignore
  const HEADER: Record<number, number> = {
    4: 7, 5: 29, 6: 31, 7: 26, 8: 24, 9: 21, 10: 19, 11: 23, 12: 32, 13: 33,
    16: 36, 17: 11, 18: 12, 19: 35, 20: 38, 22: 15, 23: 16, 24: 18,
    25: 22, 26: 37, 27: 13,
  };
  const NONE = -1;

  const KINDS = [
    { label: "Push button", value: "push" },
    { label: "Switch", value: "switch" },
  ];

  const pins = Object.entries(HEADER).map(([gpio, pos]) => ({
    label: `GPIO${gpio} (pin ${pos})`,
    value: Number(gpio),
  }));
  const ledPins = [{ label: "No light", value: NONE }, ...pins];

  let status = $state<ButtonStatus | null>(null);
  let seen = $state<number | null>(null);
  let pressedAt = $state(0);
  let now = $state(Date.now());
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh() {
    now = Date.now();
    try {
      status = await api.recordButton();
      if (seen !== null && status.presses > seen) pressedAt = now;
      seen = status.presses;
    } catch {
      // Shown as offline elsewhere
    }
  }

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 1000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  const justPressed = $derived(now - pressedAt < 3000);
</script>

<Card
  title="Record button"
  description="Wire it between the chosen pin and any ground pin."
>
  {#if status?.error}
    <p class="bg-down/10 px-4 py-3 text-[13px] text-down">{status.error}</p>
  {:else if status?.ready}
    <p class="px-4 py-3 text-[12px] {justPressed ? 'text-ok' : 'text-muted'}">
      {justPressed
        ? "Received."
        : "Listening. Press the button or flip the switch to see it register here."}
    </p>
  {/if}

  <Field label="Use a button or switch" for="btn-enabled">
    <Switch id="btn-enabled" bind:checked={button.enabled} />
  </Field>

  {#if button.enabled}
    <Field label="Type" stacked>
      <div class="w-full sm:w-64">
        <Segmented
          options={KINDS}
          value={button.kind}
          onchange={(v) => (button.kind = v)}
        />
      </div>
    </Field>

    <Field label="Pin" for="btn-pin">
      <Select id="btn-pin" options={pins} bind:value={button.pin} />
    </Field>

    <Field
      label="Recording light"
      for="btn-led"
      help="Needs a resistor of about 330 Ω."
    >
      <Select
        id="btn-led"
        options={ledPins}
        value={button.led_pin ?? NONE}
        onchange={(v) => (button.led_pin = v === NONE ? null : Number(v))}
      />
    </Field>
  {/if}
</Card>
