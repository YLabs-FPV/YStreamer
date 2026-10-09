<script lang="ts">
  /** A changelog section: "- " lines are a list, "### " lines headings */
  let { notes }: { notes: string } = $props();

  type Block =
    | { kind: "list"; items: string[] }
    | { kind: "heading" | "text"; text: string };

  const blocks = $derived.by(() => {
    const out: Block[] = [];
    for (const raw of notes.split("\n")) {
      const line = raw.trim();
      if (!line) continue;
      const item = /^[-*] (.*)$/.exec(line);
      const last = out.at(-1);
      if (item) {
        if (last?.kind === "list") last.items.push(item[1]);
        else out.push({ kind: "list", items: [item[1]] });
      } else if (line.startsWith("#")) {
        out.push({ kind: "heading", text: line.replace(/^#+\s*/, "") });
      } else if (last?.kind === "list" && /^\s/.test(raw)) {
        // a wrapped list item
        last.items[last.items.length - 1] += ` ${line}`;
      } else {
        out.push({ kind: "text", text: line });
      }
    }
    return out;
  });
</script>

<div class="flex flex-col gap-1.5 text-[12px] text-muted">
  {#each blocks as block}
    {#if block.kind === "list"}
      <ul class="list-disc pl-4">
        {#each block.items as item}
          <li>{item}</li>
        {/each}
      </ul>
    {:else if block.kind === "heading"}
      <p class="font-medium text-fg">{block.text}</p>
    {:else}
      <p>{block.text}</p>
    {/if}
  {/each}
</div>
