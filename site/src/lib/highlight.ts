import { createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

// Pages are built inside Cloudflare's runtime, which doesn't allow
// WebAssembly, so this uses Shiki's JavaScript regex engine rather than the
// default one. Only the shell language and the docs' two themes are loaded.
let highlighter: Promise<HighlighterCore> | undefined;

function getHighlighter() {
  highlighter ??= createHighlighterCore({
    themes: [
      import("shiki/themes/github-light.mjs"),
      import("shiki/themes/github-dark.mjs"),
    ],
    langs: [import("shiki/langs/shellscript.mjs")],
    engine: createJavaScriptRegexEngine(),
  });
  return highlighter;
}

/** A shell command as Shiki HTML, with light and dark colours as CSS variables */
export async function highlightCommand(command: string): Promise<string> {
  return (await getHighlighter()).codeToHtml(command, {
    lang: "shellscript",
    themes: { light: "github-light", dark: "github-dark" },
    defaultColor: false,
  });
}
