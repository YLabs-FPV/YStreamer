<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import type { SystemAction, SystemInfo } from "@/types/system";
  import { settings } from "@/stores/settings.svelte";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import DumlMonitor from "@/components/system/DumlMonitor.svelte";
  import UpdateCard from "@/components/system/UpdateCard.svelte";
  import { btn } from "@/components/ui/buttons";
  import { developer } from "@/stores/developer.svelte";
  import { auth } from "@/stores/auth.svelte";
  let {
    advanced = $bindable(),
    tab,
  }: { advanced: Settings["advanced"]; tab: string } = $props();

  let info = $state<SystemInfo | null>(null);

  interface Pending {
    title: string;
    message: string;
    confirmLabel: string;
    danger: boolean;
    run: () => void;
  }
  let pending = $state<Pending | null>(null);
  let confirmOpen = $state(false);

  function ask(p: Pending) {
    pending = p;
    confirmOpen = true;
  }

  type ServiceAction = Exclude<SystemAction, "reboot" | "poweroff">;

  const ACTIONS: Record<
    ServiceAction,
    Omit<Pending, "run"> & { done: string }
  > = {
    restart: {
      title: "Restart YStreamer?",
      message: "Video stops for a few seconds while the service restarts.",
      confirmLabel: "Restart",
      danger: false,
      done: "Restarting YStreamer…",
    },
    "restart-usb": {
      title: "Restart the USB link?",
      message:
        "The goggles link drops and reconnects. Video stops for a few seconds.",
      confirmLabel: "Restart",
      danger: false,
      done: "Restarting the USB link…",
    },
  };

  function action(a: ServiceAction) {
    const { done, ...p } = ACTIONS[a];
    ask({
      ...p,
      run: async () => {
        try {
          await api.action(a);
          snackbar.show(done);
        } catch (e) {
          snackbar.show(message(e), true);
        }
      },
    });
  }

  function exportConfig() {
    if (!settings.saved) return;
    const blob = new Blob([JSON.stringify(settings.saved, null, 2)], {
      type: "application/json",
    });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `ystreamer-${info?.hostname ?? "settings"}.json`;
    a.click();
    URL.revokeObjectURL(a.href);
  }

  let fileInput = $state<HTMLInputElement>();

  async function importConfig(file: File) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(await file.text());
    } catch {
      snackbar.show("That file isn't valid JSON", true);
      return;
    }
    ask({
      title: "Import settings?",
      message:
        "All current settings are replaced. If WiFi settings differ, the device may drop off your network.",
      confirmLabel: "Import",
      danger: false,
      run: async () => {
        try {
          const res = await api.replaceAll(parsed);
          settings.replace(res);
          snackbar.show(res.notice ?? "Settings imported");
        } catch (e) {
          snackbar.show(message(e), true);
        }
      },
    });
  }

  function reset() {
    ask({
      title: "Reset all settings?",
      message:
        "Everything goes back to defaults, including WiFi and RTSP. Camera settings live on the goggles and aren't affected.",
      confirmLabel: "Reset",
      danger: true,
      run: async () => {
        try {
          const res = await api.reset();
          settings.replace(res);
          snackbar.show(res.notice ?? "Settings reset");
        } catch (e) {
          snackbar.show(message(e), true);
        }
      },
    });
  }

  onMount(async () => {
    try {
      info = await api.system();
    } catch {}
  });

  const noSystemd = $derived(info !== null && !info.systemd);
</script>

<div class="flex flex-col gap-4">
  {#if tab === "service"}
    <Card title="Service">
      {#if noSystemd}
        <p class="px-4 py-3 text-[12px] text-muted">
          YStreamer isn't running under systemd, so it can't restart itself.
          Install
          <code class="font-mono">{info?.unit}</code> to enable restarts and logs.
        </p>
      {/if}
      <Field label="Restart YStreamer">
        <button
          type="button"
          class={btn.plain}
          disabled={!info?.systemd}
          onclick={() => action("restart")}
        >
          Restart
        </button>
      </Field>
      <Field label="Restart USB link">
        <button
          type="button"
          class={btn.plain}
          onclick={() => action("restart-usb")}
        >
          Restart
        </button>
      </Field>
    </Card>
  {:else if tab === "updates"}
    <UpdateCard bind:advanced />
  {:else if tab === "configuration"}
    <Card title="Configuration">
      <Field label="Export settings">
        <button type="button" class={btn.plain} onclick={exportConfig}
          >Export</button
        >
      </Field>
      <Field label="Import settings">
        <input
          bind:this={fileInput}
          type="file"
          accept="application/json,.json"
          class="hidden"
          onchange={(e) => {
            const f = e.currentTarget.files?.[0];
            e.currentTarget.value = "";
            if (f) importConfig(f);
          }}
        />
        <button
          type="button"
          class={btn.plain}
          onclick={() => fileInput?.click()}>Import…</button
        >
      </Field>
      <Field
        label="Reset to defaults"
        help={info ? `Stored in ${info.settings_path}` : undefined}
      >
        <button type="button" class={btn.danger} onclick={reset}>Reset</button>
      </Field>
    </Card>
  {:else if tab === "developer"}
    <Card title="Developer">
      <Field
        label="Developer mode"
        for="developer"
        help="Turn it back on by tapping the YStreamer version in System → About 7 times."
      >
        <Switch
          id="developer"
          bind:checked={() => developer.enabled, (on) => developer.set(on)}
        />
      </Field>
      <Field label="Verbose logging" for="verbose">
        <Switch id="verbose" bind:checked={advanced.verbose_logs} />
      </Field>
      <Field
        label="Terminal"
        for="terminal"
        help={auth.enabled
          ? "A shell on this device in the browser, as root."
          : "A shell on this device in the browser, as root. Set a password under System → General first."}
        disabled={!auth.enabled}
      >
        <Switch
          id="terminal"
          bind:checked={advanced.terminal}
          disabled={!auth.enabled}
        />
      </Field>
    </Card>

    <DumlMonitor />
  {/if}
</div>

{#if pending}
  <ConfirmDialog
    bind:open={confirmOpen}
    title={pending.title}
    message={pending.message}
    confirmLabel={pending.confirmLabel}
    danger={pending.danger}
    onconfirm={pending.run}
  />
{/if}
