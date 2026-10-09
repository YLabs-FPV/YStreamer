<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import type { SshStatus } from "@/types/system";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";

  const MIN_LENGTH = 8;

  let status = $state<SshStatus | null>(null);
  let busy = $state(false);
  let switching = $state<boolean | null>(null);
  let confirmOff = $state(false);

  let changing = $state(false);
  let next = $state("");
  let confirm = $state("");
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      status = await api.ssh();
    } catch {
      // The card just doesn't show
    }
  });

  async function setEnabled(on: boolean) {
    busy = true;
    switching = on;
    try {
      status = await api.setSsh(on);
      if (status.enabled === on) snackbar.show(on ? "SSH is on" : "SSH is off");
      else
        snackbar.show(on ? "SSH didn't turn on" : "SSH didn't turn off", true);
    } catch (e) {
      snackbar.show(message(e), true);
      // It may have got partway, so show what's true now
      status = await api.ssh().catch(() => status);
    } finally {
      busy = false;
      switching = null;
    }
  }

  function toggle(on: boolean) {
    if (on) setEnabled(true);
    else confirmOff = true;
  }

  const problem = $derived.by(() => {
    if (!next) return "";
    if (next.length < MIN_LENGTH)
      return `Use at least ${MIN_LENGTH} characters`;
    if (confirm && confirm !== next) return "Passwords don't match";
    return confirm ? null : "";
  });

  function openChange() {
    changing = true;
    next = confirm = "";
    error = null;
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (problem !== null || busy) return;
    busy = true;
    error = null;
    try {
      status = await api.setSshPassword(next);
      snackbar.show("SSH password changed");
      changing = false;
    } catch (err) {
      error = message(err);
    } finally {
      busy = false;
    }
  }
</script>

{#if status?.available}
  <Card title="SSH">
    {#if status.enabled && status.default_password}
      <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
        The password is still the one every YStreamer ships with, so anyone on
        the network can log in. Change it below.
      </p>
    {/if}

    <Field
      label="Allow SSH"
      for="ssh-enabled"
      help={status.user
        ? `Log in with ssh ${status.user}@${location.hostname}`
        : undefined}
    >
      <Switch
        id="ssh-enabled"
        busy={switching !== null}
        disabled={busy}
        bind:checked={
          () => switching ?? status?.enabled ?? false, (on) => toggle(on)
        }
      />
    </Field>

    <Field
      label="Password"
      help="For SSH and the device's own login, not for this page."
    >
      <button type="button" class={btn.plain} onclick={openChange}
        >Change…</button
      >
    </Field>

    {#if changing}
      <form onsubmit={submit} class="flex flex-col pb-3">
        <!-- Lets password managers file this apart from the page's own -->
        <input
          type="text"
          name="username"
          autocomplete="username"
          value="{status.user}@ssh"
          readonly
          hidden
        />
        <Field label="New password" stacked>
          <TextField secret autocomplete="new-password" bind:value={next} />
        </Field>
        <Field label="Repeat new password" stacked>
          <TextField secret autocomplete="new-password" bind:value={confirm} />
        </Field>

        {#if error || problem}
          <p class="px-4 pb-2 text-[13px] text-down">{error ?? problem}</p>
        {/if}

        <div class="flex justify-end gap-2 px-4">
          <button
            type="button"
            class={btn.text}
            onclick={() => (changing = false)}>Cancel</button
          >
          <button
            type="submit"
            class={btn.primary}
            disabled={problem !== null || busy}
          >
            {busy ? "Saving…" : "Save password"}
          </button>
        </div>
      </form>
    {/if}
  </Card>

  <ConfirmDialog
    bind:open={confirmOff}
    title="Turn SSH off?"
    message="Sessions that are open now end. After that the only way back in is this page."
    confirmLabel="Turn off"
    onconfirm={() => setEnabled(false)}
  />
{/if}
