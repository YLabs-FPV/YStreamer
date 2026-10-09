export type Res = "1080p" | "2.7K" | "4K";
export type Aspect = "16:9" | "4:3";

export const FPS = [
  { hz: 30, wire: 3 },
  { hz: 48, wire: 15 },
  { hz: 50, wire: 5 },
  { hz: 60, wire: 6 },
  { hz: 100, wire: 10 },
  { hz: 120, wire: 7 },
];

export const hzOf = (wire: number | null) =>
  FPS.find((f) => f.wire === wire)?.hz ?? null;

interface CameraModel {
  name: string;
  /** Frame rates on offer at a resolution; none means it isn't offered */
  rates: Record<Res, number[]>;
  /** Why 4:3 can't be had at this resolution and rate, if it can't */
  no43: (res: Res, hz: number) => string | null;
}

const ALL_RATES = FPS.map((f) => f.hz);

const O4_PRO: CameraModel = {
  name: "O4 Air Unit Pro",
  rates: { "1080p": ALL_RATES, "2.7K": ALL_RATES, "4K": ALL_RATES },
  no43: (_, hz) => (hz > 60 ? "4:3 needs 60 fps or less" : null),
};

const O4_LITE: CameraModel = {
  name: "O4 Air Unit",
  rates: {
    "1080p": [30, 50, 60, 100, 120],
    "2.7K": [],
    "4K": [30, 50, 60],
  },
  no43: () => null,
};

const O3_RATES = [30, 50, 60, 100, 120];
const O3: CameraModel = {
  name: "O3 Air Unit",
  rates: { "1080p": O3_RATES, "2.7K": O3_RATES, "4K": O3_RATES },
  no43: (res) => (res === "4K" ? "4:3 isn't available in 4K" : null),
};

/** Nothing is ruled out for an aircraft whose camera isn't known */
export function cameraModel(code: string | undefined): CameraModel | null {
  if (code === "ZA5305") return O4_PRO;
  if (code === "ZA530") return O4_LITE;
  if (code === "WM1695") return O3;
  return null;
}

interface Format {
  res: Res | null;
  ar: Aspect | null;
  hz: number | null;
}

const list = (rates: number[]) => rates.join(", ");

const offers = (m: CameraModel, hz: number) =>
  Object.values(m.rates).some((r) => r.includes(hz));

/** 48 fps is the O4 Pro's alone, so it isn't even listed for the others */
export const listsRate = (m: CameraModel | null, hz: number) =>
  hz !== 48 || (m !== null && offers(m, hz));

/** Why this resolution can't be picked with the rest left as it is */
export function resBlocked(m: CameraModel | null, res: Res, now: Format) {
  if (!m) return null;
  const rates = m.rates[res];
  if (!rates.length) return `The ${m.name} doesn't record in ${res}`;
  // A rate the camera is at but the table lacks means the table is wrong,
  // and that's no reason to refuse
  if (now.hz !== null && offers(m, now.hz) && !rates.includes(now.hz))
    return `${res} offers ${list(rates)} fps on the ${m.name}. Change the frame rate first.`;
  const no43 = now.ar === "4:3" && now.hz !== null && m.no43(res, now.hz);
  return no43 ? `${no43} on the ${m.name}. Switch to 16:9 first.` : null;
}

/** Why this frame rate can't be picked with the rest left as it is */
export function rateBlocked(m: CameraModel | null, hz: number, now: Format) {
  if (!m) return null;
  if (!offers(m, hz)) return `The ${m.name} has no ${hz} fps`;
  if (now.res !== null && !m.rates[now.res].includes(hz))
    return `${now.res} offers ${list(m.rates[now.res])} fps on the ${m.name}. Change the resolution first.`;
  const no43 = now.ar === "4:3" && now.res !== null && m.no43(now.res, hz);
  return no43 ? `${no43} on the ${m.name}. Switch to 16:9 first.` : null;
}

/** Why this aspect ratio can't be picked with the rest left as it is */
export function aspectBlocked(m: CameraModel | null, ar: Aspect, now: Format) {
  if (!m || ar !== "4:3" || now.res === null || now.hz === null) return null;
  const no43 = m.no43(now.res, now.hz);
  return no43 ? `${no43} on the ${m.name}` : null;
}

/** 1/100 and faster always work; slower ones only down to the frame time */
export const SHUTTERS = [
  30, 40, 50, 60, 80, 100, 120, 160, 200, 250, 320, 400, 500, 640, 800, 1000,
  1250, 1600, 2000, 2500, 3200, 4000, 5000, 6400, 8000,
];

export function shutterBlocked(speed: number, hz: number | null) {
  if (speed === 0 || speed >= 100 || hz === null || speed >= hz) return null;
  return `1/${speed} needs ${speed} fps or less; the camera is at ${hz} fps`;
}
