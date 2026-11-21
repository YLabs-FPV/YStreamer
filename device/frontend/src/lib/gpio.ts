// GPIO21 (pin 40) is left out: grounding it while starting resets the settings
/** Free GPIOs and where they are on the header */
// prettier-ignore
export const HEADER: Record<number, number> = {
  4: 7, 5: 29, 6: 31, 7: 26, 8: 24, 9: 21, 10: 19, 11: 23, 12: 32, 13: 33,
  16: 36, 17: 11, 18: 12, 19: 35, 20: 38, 22: 15, 23: 16, 24: 18,
  25: 22, 26: 37, 27: 13,
};

export const pinLabel = (gpio: number) => `GPIO${gpio} (pin ${HEADER[gpio]})`;

export const PINS = Object.keys(HEADER).map((gpio) => ({
  label: pinLabel(Number(gpio)),
  value: Number(gpio),
}));

export const BUTTON_KINDS = [
  { label: "Push button", value: "push" },
  { label: "Switch", value: "switch" },
];
