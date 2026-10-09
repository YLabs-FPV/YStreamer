<script lang="ts">
  import { message } from "@/api/client";
  import { auth } from "@/stores/auth.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import { btn } from "@/components/ui/buttons";
  import Lock from "@lucide/svelte/icons/lock";

  let { reason }: { reason: string } = $props();

  let password = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!password || busy) return;
    busy = true;
    error = null;
    try {
      await auth.login(password);
      password = "";
    } catch (err) {
      error = message(err);
    } finally {
      busy = false;
    }
  }
</script>

<form
  onsubmit={submit}
  class="mx-auto flex w-full max-w-sm flex-col gap-4 rounded-xl border border-line bg-surface p-6 shadow-sm"
>
  <div class="flex items-center gap-3">
    <span
      class="grid h-9 w-9 shrink-0 place-items-center rounded-full bg-surface-2"
    >
      <Lock class="h-4 w-4 text-muted" aria-hidden="true" />
    </span>
    <div>
      <h2 class="text-[15px] font-semibold text-fg">Log in</h2>
      <p class="text-[12px] text-muted">{reason}</p>
    </div>
  </div>

  <!-- Lets password managers file the password under this device -->
  <input
    type="text"
    name="username"
    autocomplete="username"
    value="ystreamer"
    readonly
    hidden
  />
  <label class="flex flex-col gap-1.5 text-[13px] text-fg">
    Password
    <TextField
      secret
      name="password"
      autocomplete="current-password"
      bind:value={password}
      class="sm:w-full!"
    />
  </label>

  {#if error}
    <p class="text-[13px] text-down">{error}</p>
  {/if}

  <button type="submit" class={btn.primary} disabled={!password || busy}>
    {busy ? "Checking…" : "Log in"}
  </button>
</form>
