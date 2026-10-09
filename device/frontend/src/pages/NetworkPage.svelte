<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, message } from "@/api/client";
  import type {
    NetworkStatus,
    ScanResult,
    WifiCountries,
  } from "@/types/network";
  import type { Settings } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import TailscaleCard from "@/components/network/TailscaleCard.svelte";
  import WireguardCard from "@/components/network/WireguardCard.svelte";
  import { btn } from "@/components/ui/buttons";
  let {
    wifi = $bindable(),
    ethernet = $bindable(),
    tab,
  }: {
    wifi: Settings["wifi"];
    ethernet: Settings["ethernet"];
    tab: string;
  } = $props();

  const ETHERNET_MODES = [
    { label: "System", value: "system" },
    { label: "Automatic", value: "auto" },
    { label: "Static", value: "static" },
  ];

  const ETHERNET_HELP: Record<string, string> = {
    system:
      "YStreamer leaves Ethernet alone and uses whatever the OS is set up with.",
    auto: "Get an address from the network's router (DHCP).",
    static:
      "Use a fixed address. If you're connected over Ethernet, make sure it's on your network, or you'll need WiFi to get back in.",
  };

  const ethernetNow = $derived.by(() => {
    const e = status?.ethernet;
    if (!e) return null;
    if (e.state !== "connected") return `Right now: ${e.state}`;
    const how =
      e.method === "manual"
        ? "static"
        : e.method === "auto"
          ? "DHCP"
          : e.method;
    const parts = [
      how,
      e.addresses.join(", "),
      e.gateway && `gateway ${e.gateway}`,
    ];
    return `Right now: ${parts.filter(Boolean).join(" · ")}`;
  });

  // Off still starts the AP by itself when the device can't be reached
  const AP_CHOICES = [
    { label: "Off", value: false },
    { label: "On", value: true },
  ];

  const apOn = $derived(wifi.mode === "ap");
  const joining = $derived(wifi.mode === "client");

  const apHelp = $derived(
    apOn
      ? "Always broadcast YStreamer's own 2.4 GHz network. Phones and laptops connect to it directly."
      : "Only starts by itself when the fallback below is on.",
  );

  const fallbackHelp = $derived(
    !wifi.fallback_ap
      ? "Off: without Ethernet or a joined network, this page can't be reached."
      : joining
        ? "Start the access point if the network below can't be joined within a minute, so you can always reach this page."
        : "Start the access point when there's no Ethernet or WiFi connection, e.g. unplugged in the field. It stops again once it's not needed.",
  );

  let confirmNoFallback = $state(false);

  function setFallback(on: boolean) {
    if (on) wifi.fallback_ap = true;
    else confirmNoFallback = true;
  }

  function setAp(on: boolean) {
    wifi.mode = on ? "ap" : wifi.client.ssid ? "client" : "unmanaged";
  }

  function setJoining(on: boolean) {
    wifi.mode = on ? "client" : "unmanaged";
  }

  const CHANNELS = Array.from({ length: 11 }, (_, i) => ({
    label: `${i + 1}  ·  ${2412 + i * 5} MHz`,
    value: i + 1,
  }));

  let status = $state<NetworkStatus | null>(null);

  const ethernetModes = $derived(
    status?.image
      ? ETHERNET_MODES.filter((m) => m.value !== "system")
      : ETHERNET_MODES,
  );
  let timer: ReturnType<typeof setInterval> | null = null;

  let countries = $state<WifiCountries | null>(null);
  const countryOptions = $derived([
    {
      label: countries?.current
        ? `Not set (the system uses ${countries.current})`
        : "Not set",
      value: "",
    },
    ...(countries?.countries ?? []).map((c) => ({
      label: c.name,
      value: c.code,
    })),
  ]);

  let scanning = $state(false);
  let networks = $state<ScanResult[] | null>(null);

  async function refresh() {
    try {
      status = await api.network();
    } catch {
      // Expected while WiFi is being switched
    }
  }

  async function scan() {
    scanning = true;
    try {
      networks = await api.scan();
    } catch (e) {
      snackbar.show(`Scan failed: ${message(e)}`, true);
    } finally {
      scanning = false;
    }
  }

  function pick(n: ScanResult) {
    if (n.ssid !== wifi.client.ssid) wifi.client.password = "";
    wifi.client.ssid = n.ssid;
    networks = null;
    document.getElementById("wifi-password")?.focus();
  }

  const stateText = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

  const internet = $derived.by(() => {
    const dev = status?.internet_via;
    if (!status || !dev) return "No internet connection";
    if (dev === status.tether?.device)
      return "Internet through the phone over USB";
    if (dev === status.ethernet?.device) return "Internet through Ethernet";
    if (dev === status.wifi.device) return "Internet through WiFi";
    return `Internet through ${dev}`;
  });
  const bars = (signal: number) => (signal >= 70 ? 3 : signal >= 45 ? 2 : 1);

  onMount(() => {
    refresh();
    api
      .countries()
      .then((c) => (countries = c))
      .catch(() => {});
    timer = setInterval(refresh, 4000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });
</script>

<div class="flex flex-col gap-4">
  {#if tab === "connectivity"}
    <Card title="Status">
      {#if !status}
        <p class="px-4 py-3 text-[13px] text-muted">Loading…</p>
      {:else}
        {#if status.fallback_active}
          <p
            class="px-4 py-3 text-[13px] {status.fallback_resolved
              ? 'bg-surface-2 text-muted'
              : 'bg-warn/10 text-fg'}"
          >
            {status.fallback_reason ?? "The fallback access point is on."}
            {#if !status.fallback_resolved}
              It stays on until the WiFi settings are saved again or the device
              restarts.
            {/if}
          </p>
        {/if}
        {#if status.last_error}
          <p class="bg-down/10 px-4 py-3 text-[13px] text-down">
            {status.last_error}
          </p>
        {/if}

        {@render iface("WiFi", status.wifi)}
        {#if status.ethernet}
          {@render iface("Ethernet", status.ethernet)}
        {/if}
        {#if status.tether}
          {@render iface(
            status.tether.product
              ? `Phone over USB (${status.tether.product})`
              : "Phone over USB",
            status.tether,
          )}
        {/if}
        <p class="px-4 py-2.5 text-[12px] text-muted">{internet}</p>
      {/if}
    </Card>

    <Card
      title="WiFi"
      description="The radio either broadcasts its own network or joins one, not both."
    >
      <Field
        label="Country"
        for="wifi-country"
        help={!wifi.country && status?.image && !countries?.current
          ? "Choose where the device is used, so the radio follows the local rules."
          : "Sets which channels and power the radio may use."}
      >
        <Select
          id="wifi-country"
          bind:value={wifi.country}
          options={countryOptions}
        />
      </Field>

      <div class="flex flex-col gap-2 px-4 py-3">
        <span class="text-[13px] text-fg">Access point</span>
        <Segmented
          options={AP_CHOICES}
          value={apOn}
          onchange={(v) => setAp(v)}
        />
        <p class="text-[12px] text-muted">{apHelp}</p>
      </div>

      {#if apOn}
        <Field label="Network name" for="ap-ssid">
          <TextField id="ap-ssid" bind:value={wifi.ap.ssid} maxlength={32} />
        </Field>
        <Field
          label="Password"
          for="ap-password"
          help="WPA2, 8 to 63 characters."
        >
          <TextField
            id="ap-password"
            secret
            bind:value={wifi.ap.password}
            minlength={8}
            maxlength={63}
          />
        </Field>
        <Field label="Channel" for="ap-channel">
          <Select
            id="ap-channel"
            options={CHANNELS}
            bind:value={wifi.ap.channel}
          />
        </Field>
        <Field
          label="Address"
          for="ap-address"
          help="Devices on the access point open http://{wifi.ap.address ||
            '…'}:{location.port || 80}"
        >
          <TextField id="ap-address" mono bind:value={wifi.ap.address} />
        </Field>
      {:else}
        <Field label="Fallback" for="wifi-fallback" help={fallbackHelp}>
          <Switch
            id="wifi-fallback"
            bind:checked={() => wifi.fallback_ap, (on) => setFallback(on)}
          />
        </Field>
      {/if}

      <Field
        label="Join a network"
        for="wifi-join"
        help={apOn
          ? "Turn the access point off to join a network instead."
          : joining
            ? undefined
            : status?.image
              ? "Off: WiFi doesn't join any network."
              : "Off: WiFi is left to the system, using whatever the OS is set up with."}
        disabled={apOn}
      >
        <Switch
          id="wifi-join"
          bind:checked={() => joining, (on) => setJoining(on)}
          disabled={apOn}
        />
      </Field>

      {#if joining}
        <Field
          label="Network name"
          for="wifi-ssid"
          help="2.4 GHz only: 5 GHz is where the goggles' video link runs."
        >
          <TextField
            id="wifi-ssid"
            bind:value={wifi.client.ssid}
            maxlength={32}
          />
          <button
            type="button"
            class={btn.plain}
            onclick={scan}
            disabled={scanning}
          >
            {scanning ? "Scanning…" : "Scan"}
          </button>
        </Field>

        {#if networks}
          <div class="px-4 py-2">
            {#if networks.length === 0}
              <p class="py-1 text-[13px] text-muted">
                No 2.4 GHz networks found.
              </p>
            {:else}
              <ul
                class="max-h-64 overflow-y-auto rounded-md border border-line"
              >
                {#each networks as n (n.ssid)}
                  <li class="border-t border-line first:border-t-0">
                    <button
                      type="button"
                      onclick={() => pick(n)}
                      class="flex w-full items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-surface-2 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary"
                    >
                      <svg
                        viewBox="0 0 16 16"
                        class="h-4 w-4 shrink-0"
                        aria-label="Signal {n.signal}%"
                      >
                        {#each [1, 2, 3] as b}
                          <rect
                            x={1 + (b - 1) * 5}
                            y={13 - b * 4}
                            width="3.5"
                            height={b * 4}
                            rx="0.75"
                            class={b <= bars(n.signal)
                              ? "fill-fg"
                              : "fill-surface-3"}
                          />
                        {/each}
                      </svg>
                      <span class="min-w-0 flex-1 truncate text-[13px] text-fg"
                        >{n.ssid}</span
                      >
                      <span class="text-[12px] text-faint">
                        {n.security || "Open"}
                      </span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {/if}

        <Field
          label="Password"
          for="wifi-password"
          help="Leave empty for an open network."
        >
          <TextField
            id="wifi-password"
            secret
            bind:value={wifi.client.password}
            maxlength={63}
          />
        </Field>
      {/if}
    </Card>

    <Card title="Phone over USB">
      <div class="flex flex-col gap-2 px-4 py-3 text-[13px] text-muted">
        <p>Plug in a phone with USB tethering on; nothing to set here.</p>
        {#if status && !status.iphone_support}
          <p class="text-warn">
            iPhones need usbmuxd, which isn't installed. Run <code
              class="font-mono text-fg">sudo apt install usbmuxd</code
            >.
          </p>
        {/if}
      </div>
    </Card>

    <Card title="Ethernet">
      <div class="flex flex-col gap-2 px-4 py-3">
        <Segmented
          options={ethernetModes}
          value={ethernet.mode}
          onchange={(v) => (ethernet.mode = v)}
        />
        <p class="text-[12px] text-muted">{ETHERNET_HELP[ethernet.mode]}</p>
        {#if ethernetNow}
          <p class="font-mono text-[12px] text-faint">{ethernetNow}</p>
        {/if}
      </div>

      {#if ethernet.mode === "static"}
        <Field
          label="Address"
          for="eth-address"
          help="With the network size, e.g. 192.168.1.50/24."
        >
          <TextField
            id="eth-address"
            mono
            placeholder="192.168.1.50/24"
            bind:value={ethernet.address}
          />
        </Field>
        <Field label="Gateway" for="eth-gateway">
          <TextField
            id="eth-gateway"
            mono
            placeholder="192.168.1.1"
            bind:value={ethernet.gateway}
          />
        </Field>
        <Field
          label="DNS servers"
          for="eth-dns"
          help="Separated by commas. Leave empty for none."
        >
          <TextField
            id="eth-dns"
            mono
            placeholder="1.1.1.1, 8.8.8.8"
            bind:value={ethernet.dns}
          />
        </Field>
      {/if}
    </Card>
  {:else if tab === "remote"}
    <TailscaleCard />

    <WireguardCard />
  {/if}
</div>

<ConfirmDialog
  bind:open={confirmNoFallback}
  title="Turn off the fallback?"
  message="If Ethernet is unplugged and no WiFi network is joined, e.g. in the field, there will be no way to reach this page until you plug in Ethernet."
  confirmLabel="Turn off"
  danger
  onconfirm={() => (wifi.fallback_ap = false)}
/>

{#snippet iface(name: string, s: NetworkStatus["wifi"])}
  <div
    class="flex flex-col gap-1 px-4 py-3 sm:flex-row sm:items-baseline sm:justify-between sm:gap-4"
  >
    <div class="flex items-center gap-2">
      <span
        class="h-1.5 w-1.5 rounded-full {s.state === 'connected'
          ? 'bg-ok'
          : s.state.startsWith('connecting')
            ? 'bg-warn'
            : 'bg-faint'}"
      ></span>
      <span class="text-[13px] text-fg">{name}</span>
      <span class="text-[12px] text-muted">
        {stateText(s.state)}{#if s.role === "ap"}&nbsp;as access point{/if}
      </span>
    </div>
    <div class="flex flex-wrap gap-x-3 pl-3.5 text-[12px] text-muted sm:pl-0">
      {#if s.ssid}<span>{s.ssid}</span>{/if}
      {#each s.addresses as a}<span class="font-mono">{a}</span>{/each}
    </div>
  </div>
{/snippet}
