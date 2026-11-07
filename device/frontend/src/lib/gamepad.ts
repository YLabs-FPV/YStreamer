// Gamepads in the browser's "standard" layout: right stick on axes 2 and 3,
// A / Cross is button 0, B / Circle 1, Y / Triangle 3

const DEADZONE = 0.15;

export const BUTTON = { center: 0, release: 1, hold: 3 } as const;

/** The first pad the browser has seen a button press from */
export function firstPad(): Gamepad | null {
  const pads = navigator.getGamepads?.() ?? [];
  return pads.find((p): p is Gamepad => !!p?.connected) ?? null;
}

/** -1..1 with the middle ignored, and up as positive */
export function rightStick(pad: Gamepad): { x: number; y: number } {
  return { x: shape(pad.axes[2] ?? 0), y: -shape(pad.axes[3] ?? 0) };
}

function shape(v: number): number {
  const a = Math.abs(v);
  if (a < DEADZONE) return 0;
  return (Math.sign(v) * (a - DEADZONE)) / (1 - DEADZONE);
}

/** Tells a new press from a button that's still held down */
export class Presses {
  #down = new Set<number>();

  pressed(pad: Gamepad, button: number): boolean {
    const down = !!pad.buttons[button]?.pressed;
    const was = this.#down.has(button);
    if (down) this.#down.add(button);
    else this.#down.delete(button);
    return down && !was;
  }
}
