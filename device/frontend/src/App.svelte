<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import "@/styles/app.css";
  import { control } from "@/stores/control.svelte";
  import { settings } from "@/stores/settings.svelte";
  import { auth } from "@/stores/auth.svelte";
  import { routeOf } from "@/lib/nav";
  import { route as address } from "@/stores/route.svelte";
  import LoginForm from "@/components/auth/LoginForm.svelte";
  import Shell from "@/components/layout/Shell.svelte";
  import PageHeader from "@/components/layout/PageHeader.svelte";
  import StartupScreen from "@/components/layout/StartupScreen.svelte";
  import Unreachable from "@/components/layout/Unreachable.svelte";
  import Snackbar from "@/components/layout/Snackbar.svelte";
  import WatchPage from "@/pages/WatchPage.svelte";
  import SettingsView from "@/pages/SettingsView.svelte";

  const route = $derived(routeOf(address.path));
  const page = $derived(route.page);
  // Reached from the sidebar by someone who's only watching
  const wantsLogin = $derived(address.path === "/login");
  const isWatchPage = $derived(page.id === "watch" && !wantsLogin);

  $effect(() => {
    if (wantsLogin && auth.loaded && auth.admin) address.go("/", true);
  });

  let theater = $state(false);

  function onbeforeunload(e: BeforeUnloadEvent) {
    if (settings.dirty.length) e.preventDefault();
  }

  function connect() {
    control.disconnect();
    if (auth.canView) control.connect();
    if (auth.admin) settings.load();
  }

  onMount(async () => {
    await auth.load();
    connect();
    auth.onChange(connect);
  });
  onDestroy(() => {
    control.disconnect();
  });
</script>

<svelte:window {onbeforeunload} />

{#if !auth.loaded}
  {#if auth.unreachable}
    <StartupScreen>
      <Unreachable />
    </StartupScreen>
  {/if}
{:else if !auth.canView}
  <StartupScreen>
    <LoginForm reason="This device is password protected." />
  </StartupScreen>
{:else}
  <Shell
    current={wantsLogin ? "login" : page.id}
    width={isWatchPage ? (theater ? "max-w-none" : "max-w-6xl") : "max-w-4xl"}
  >
    {#if wantsLogin}
      <PageHeader title="Log in" />
      <LoginForm reason="Log in to change settings and control the camera." />
    {:else if isWatchPage}
      <WatchPage title={page.label} bind:theater />
    {:else}
      <PageHeader title={page.label} />
      {#if auth.admin}
        <SettingsView {page} tab={route.tab} />
      {:else}
        <LoginForm reason="Log in to change settings." />
      {/if}
    {/if}
  </Shell>
{/if}

<Snackbar raised={!isWatchPage && settings.dirty.length > 0} />
