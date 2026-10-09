<script lang="ts">
  import { api, message } from "@/api/client";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";

  type Power = "reboot" | "poweroff";

  const ACTIONS: Record<
    Power,
    { title: string; message: string; confirmLabel: string; done: string }
  > = {
    reboot: {
      title: "Reboot the device?",
      message: "Everything stops for about a minute while it boots.",
      confirmLabel: "Reboot",
      done: "Rebooting. This page reconnects when it's back.",
    },
    poweroff: {
      title: "Power off the device?",
      message: "You'll need physical access to turn it back on.",
      confirmLabel: "Power off",
      done: "Powering off. Wait for the green LED to stop before unplugging.",
    },
  };

  let pending = $state<Power | null>(null);
  let confirmOpen = $state(false);

  function ask(a: Power) {
    pending = a;
    confirmOpen = true;
  }

  async function run() {
    if (!pending) return;
    const a = pending;
    try {
      await api.action(a);
      snackbar.show(ACTIONS[a].done);
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }
</script>

<Card title="Power">
  <Field label="Reboot device">
    <button type="button" class={btn.danger} onclick={() => ask("reboot")}
      >Reboot</button
    >
  </Field>
  <Field label="Power off device">
    <button type="button" class={btn.danger} onclick={() => ask("poweroff")}
      >Power off</button
    >
  </Field>
</Card>

{#if pending}
  <ConfirmDialog
    bind:open={confirmOpen}
    title={ACTIONS[pending].title}
    message={ACTIONS[pending].message}
    confirmLabel={ACTIONS[pending].confirmLabel}
    danger
    onconfirm={run}
  />
{/if}
