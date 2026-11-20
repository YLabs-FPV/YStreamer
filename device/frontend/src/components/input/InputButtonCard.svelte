<script lang="ts">
  import { api } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import Card from "@/components/ui/Card.svelte";
  import ButtonStatus from "@/components/ui/ButtonStatus.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import { BUTTON_KINDS, PINS } from "@/lib/gpio";
  import { settings } from "@/stores/settings.svelte";

  let {
    button = $bindable(),
    sources,
  }: {
    button: Settings["input"]["button"];
    sources: { label: string; value: string }[];
  } = $props();

  // The record button's pins, so they aren't picked twice
  const taken = $derived.by(() => {
    const record = settings.draft?.recording.button;
    if (!record?.enabled) return new Set<number>();
    return new Set(
      [record.pin, record.led_pin].filter((p): p is number => p != null),
    );
  });
  const pins = $derived(
    PINS.map((p) =>
      taken.has(p.value) ? { ...p, label: `${p.label}, record button` } : p,
    ),
  );

  const label = (mode: string) =>
    sources.find((s) => s.value === mode)?.label ?? mode;
</script>

<Card
  title="Input button"
  description="Swap between two inputs with a button or switch. Wire it between the chosen pin and any ground pin."
>
  <ButtonStatus load={api.inputButton} />

  <Field label="Use a button or switch" for="input-btn-enabled">
    <Switch id="input-btn-enabled" bind:checked={button.enabled} />
  </Field>

  {#if button.enabled}
    <Field label="Type" stacked>
      <div class="w-full sm:w-64">
        <Segmented
          options={BUTTON_KINDS}
          value={button.kind}
          onchange={(v) => (button.kind = v as typeof button.kind)}
        />
      </div>
    </Field>

    <Field label="Pin" for="input-btn-pin">
      <Select id="input-btn-pin" options={pins} bind:value={button.pin} />
    </Field>
  {/if}

  <Field
    label={button.enabled && button.kind === "switch"
      ? "Switch open"
      : "First input"}
    for="input-btn-first"
  >
    <Select id="input-btn-first" options={sources} bind:value={button.first} />
  </Field>

  <Field
    label={button.enabled && button.kind === "switch"
      ? "Switch closed"
      : "Second input"}
    for="input-btn-second"
    help={button.first === button.second
      ? "Pick two different inputs."
      : `The API's toggle also swaps between ${label(button.first)} and ${label(button.second)}.`}
  >
    <Select
      id="input-btn-second"
      options={sources}
      bind:value={button.second}
    />
  </Field>
</Card>
