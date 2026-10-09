/** Copy text, also on the plain-HTTP page a device serves, where browsers
 *  withhold the clipboard API. Throws when neither way works */
export async function copyText(text: string): Promise<void> {
  if (navigator.clipboard && window.isSecureContext) {
    await navigator.clipboard.writeText(text);
    return;
  }

  // The old way: select it in a field off screen and ask for a copy
  const field = document.createElement("textarea");
  field.value = text;
  field.setAttribute("readonly", "");
  field.style.position = "fixed";
  field.style.left = "-9999px";
  field.style.top = "0";
  document.body.appendChild(field);
  const focused = document.activeElement as HTMLElement | null;
  try {
    field.select();
    field.setSelectionRange(0, text.length);
    if (!document.execCommand("copy")) throw new Error("copy refused");
  } finally {
    field.remove();
    focused?.focus();
  }
}
