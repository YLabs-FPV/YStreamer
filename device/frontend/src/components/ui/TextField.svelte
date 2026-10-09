<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";
  import Eye from "@lucide/svelte/icons/eye";
  import EyeOff from "@lucide/svelte/icons/eye-off";

  interface Props extends Omit<HTMLInputAttributes, "value"> {
    value: string | number;
    secret?: boolean;
    mono?: boolean;
  }

  let {
    value = $bindable(),
    secret = false,
    mono = false,
    type = "text",
    class: extra = "",
    ...rest
  }: Props = $props();

  let reveal = $state(false);
</script>

<div class="relative w-full sm:w-64 {extra}">
  <input
    autocomplete="off"
    {...rest}
    type={secret ? (reveal ? "text" : "password") : type}
    bind:value
    autocapitalize="off"
    spellcheck="false"
    class="w-full rounded-md border border-line bg-surface-2 px-3 py-1.5 text-[13px] text-fg transition-colors outline-none placeholder:text-faint hover:border-line-strong focus:border-primary disabled:pointer-events-none
      {secret ? 'pr-9' : ''} {mono ? 'font-mono' : ''}"
  />
  {#if secret}
    <!-- Out of the Tab order: Tab from a password goes to the next field -->
    <button
      type="button"
      tabindex="-1"
      onclick={() => (reveal = !reveal)}
      aria-label={reveal ? "Hide password" : "Show password"}
      title={reveal ? "Hide password" : "Show password"}
      class="absolute inset-y-0 right-0 grid w-9 place-items-center text-faint transition-colors hover:text-fg focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary"
    >
      {#if reveal}
        <EyeOff class="h-4 w-4" aria-hidden="true" />
      {:else}
        <Eye class="h-4 w-4" aria-hidden="true" />
      {/if}
    </button>
  {/if}
</div>
