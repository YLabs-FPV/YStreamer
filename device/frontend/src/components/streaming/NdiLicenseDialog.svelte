<script lang="ts">
  import { btn } from "@/components/ui/buttons";

  interface Props {
    open: boolean;
    version: string;
    /** From inside the package NDI's server or the owner provided */
    license: string;
    installing: boolean;
    onaccept: () => void;
    ondecline: () => void;
  }

  let {
    open = $bindable(),
    version,
    license,
    installing,
    onaccept,
    ondecline,
  }: Props = $props();

  const NDI = "https://ndi.video/";

  let dialog: HTMLDialogElement;

  $effect(() => {
    if (open && !dialog.open) dialog.showModal();
    else if (!open && dialog.open) dialog.close();
  });
</script>

<!-- Closing it answers nothing: the download waits on the card -->
<dialog
  bind:this={dialog}
  onclose={() => (open = false)}
  oncancel={(e) => installing && e.preventDefault()}
  onclick={(e) => e.target === dialog && !installing && dialog.close()}
  aria-labelledby="ndi-license-title"
  class="m-auto flex max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-2xl flex-col rounded-xl border border-line bg-surface p-0 text-fg shadow-lg backdrop:bg-black/50 [&:not([open])]:hidden"
>
  <div class="px-5 pt-5 pb-3">
    <h2 id="ndi-license-title" class="text-[15px] font-medium">
      NDI's licence (runtime {version})
    </h2>
    <p class="mt-2 text-[13px] leading-relaxed text-muted">
      NDI's runtime is NDI's own software. Read its licence: nothing is
      installed unless you accept it, and declining deletes the download.
      <a class="underline" href={NDI} target="_blank" rel="noreferrer"
        >ndi.video</a
      >
    </p>
  </div>
  <pre
    class="mx-5 min-h-0 flex-1 overflow-auto rounded-lg bg-surface-2 p-3 font-mono text-[11px] whitespace-pre-wrap text-muted">{license}</pre>
  <div class="flex justify-end gap-2 px-5 py-4">
    <button
      type="button"
      class={btn.text}
      disabled={installing}
      onclick={() => {
        dialog.close();
        ondecline();
      }}
    >
      Decline
    </button>
    <button
      type="button"
      class={btn.primary}
      disabled={installing}
      onclick={onaccept}
    >
      {installing ? "Installing…" : "Accept and install"}
    </button>
  </div>
</dialog>
