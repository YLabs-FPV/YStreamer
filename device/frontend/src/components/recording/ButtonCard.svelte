<script lang="ts">
  import { api } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import Card from "@/components/ui/Card.svelte";
  import ButtonStatus from "@/components/ui/ButtonStatus.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import { BUTTON_KINDS as KINDS, PINS } from "@/lib/gpio";

  let { button = $bindable() }: { button: Settings["recording"]["button"] } =
    $props();

  const NONE = -1;

  const pins = PINS;
  const ledPins = [{ label: "No light", value: NONE }, ...pins];
</script>

<Card
  title="Record button"
  description="Wire it between the chosen pin and any ground pin."
>
  <ButtonStatus load={api.recordButton} />

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
