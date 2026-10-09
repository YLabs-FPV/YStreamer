<script lang="ts">
  import { message } from "@/api/client";
  import { auth } from "@/stores/auth.svelte";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import { btn } from "@/components/ui/buttons";

  const MIN_LENGTH = 6;

  let mode = $state<"set" | "change" | "remove" | null>(null);
  let current = $state("");
  let next = $state("");
  let confirm = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  function open(m: typeof mode) {
    mode = m;
    current = next = confirm = "";
    error = null;
  }

  const problem = $derived.by(() => {
    if (mode === "remove") return current ? null : "";
    if (mode === "change" && !current) return "";
    if (!next) return "";
    if (next.length < MIN_LENGTH)
      return `Use at least ${MIN_LENGTH} characters`;
    if (confirm && confirm !== next) return "Passwords don't match";
    return confirm ? null : "";
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (problem !== null || busy) return;
    busy = true;
    error = null;
    try {
      await auth.setPassword(current, mode === "remove" ? null : next);
      snackbar.show(
        mode === "remove"
          ? "Password removed"
          : mode === "set"
            ? "Password set. Other browsers need to log in."
            : "Password changed. Other browsers are logged out.",
      );
      mode = null;
    } catch (err) {
      error = message(err);
    } finally {
      busy = false;
    }
  }

  async function toggleViewing(protect: boolean) {
    try {
      await auth.setProtectViewing(protect);
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }
</script>

<Card title="Access">
  <Field
    label="Password"
    help={auth.enabled
      ? "Changing anything needs a login. Forgot it? Delete /etc/ystreamer/auth.json on the device."
      : "Not set: anyone on the network can change settings."}
  >
    <div class="flex gap-2">
      {#if auth.enabled}
        <button type="button" class={btn.plain} onclick={() => open("change")}
          >Change…</button
        >
        <button type="button" class={btn.danger} onclick={() => open("remove")}
          >Remove…</button
        >
      {:else}
        <button type="button" class={btn.plain} onclick={() => open("set")}
          >Set password…</button
        >
      {/if}
    </div>
  </Field>

  {#if mode}
    <form onsubmit={submit} class="flex flex-col pb-3">
      <!-- Lets password managers file the password under this device -->
      <input
        type="text"
        name="username"
        autocomplete="username"
        value="ystreamer"
        readonly
        hidden
      />
      {#if mode !== "set"}
        <Field label="Current password" stacked>
          <TextField
            secret
            autocomplete="current-password"
            bind:value={current}
          />
        </Field>
      {/if}
      {#if mode !== "remove"}
        <Field label="New password" stacked>
          <TextField secret autocomplete="new-password" bind:value={next} />
        </Field>
        <Field label="Repeat new password" stacked>
          <TextField secret autocomplete="new-password" bind:value={confirm} />
        </Field>
      {/if}

      {#if error || problem}
        <p class="px-4 pb-2 text-[13px] text-down">{error ?? problem}</p>
      {/if}

      <div class="flex justify-end gap-2 px-4">
        <button type="button" class={btn.text} onclick={() => (mode = null)}
          >Cancel</button
        >
        <button
          type="submit"
          class={mode === "remove" ? btn.danger : btn.primary}
          disabled={problem !== null || busy}
        >
          {busy
            ? "Saving…"
            : mode === "remove"
              ? "Remove password"
              : "Save password"}
        </button>
      </div>
    </form>
  {/if}

  <Field
    label="Require password to watch"
    for="protect-viewing"
    help={auth.enabled
      ? "Off: anyone on the network can watch the live view, but not change anything."
      : "Set a password first."}
    disabled={!auth.enabled}
  >
    <Switch
      id="protect-viewing"
      disabled={!auth.enabled}
      bind:checked={() => auth.protectViewing, (on) => toggleViewing(on)}
    />
  </Field>
</Card>
