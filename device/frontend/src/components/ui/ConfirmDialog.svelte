<script lang="ts">
  import { btn } from "@/components/ui/buttons";
  interface Props {
    open: boolean;
    title: string;
    message: string;
    confirmLabel?: string;
    danger?: boolean;
    onconfirm: () => void;
  }

  let {
    open = $bindable(),
    title,
    message,
    confirmLabel = "Continue",
    danger = false,
    onconfirm,
  }: Props = $props();

  let dialog: HTMLDialogElement;

  $effect(() => {
    if (open && !dialog.open) dialog.showModal();
    else if (!open && dialog.open) dialog.close();
  });
</script>

<dialog
  bind:this={dialog}
  onclose={() => (open = false)}
  onclick={(e) => e.target === dialog && dialog.close()}
  class="m-auto w-[calc(100%-2rem)] max-w-sm rounded-xl border border-line bg-surface p-0 text-fg shadow-lg backdrop:bg-black/50"
>
  <div class="px-5 pt-5 pb-4">
    <h2 class="text-[15px] font-medium">{title}</h2>
    <p class="mt-2 text-[13px] leading-relaxed text-muted">{message}</p>
  </div>
  <div class="flex justify-end gap-2 px-5 pb-5">
    <button type="button" class={btn.text} onclick={() => dialog.close()}
      >Cancel</button
    >
    <button
      type="button"
      class={danger ? btn.danger : btn.primary}
      onclick={() => {
        dialog.close();
        onconfirm();
      }}
    >
      {confirmLabel}
    </button>
  </div>
</dialog>
