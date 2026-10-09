const base =
  "inline-flex items-center justify-center gap-1.5 rounded-md px-3 py-1.5 text-[13px] font-medium transition-colors disabled:pointer-events-none disabled:opacity-45 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary";

export const btn = {
  plain: `${base} border border-line bg-surface-2 text-muted hover:bg-surface-3 hover:text-fg`,
  primary: `${base} bg-primary text-white hover:bg-primary-hover`,
  danger: `${base} border border-line bg-surface-2 text-down hover:border-down hover:bg-down hover:text-white`,
  text: `${base} text-muted hover:bg-surface-2 hover:text-fg`,
};
