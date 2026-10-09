export const SITE_TITLE = "YStreamer";
export const AUTHOR = "YarosFPV";

// English fallback description. Per-locale copy lives in src/i18n/locales/*.json
// (site.description) and is what the pages actually render.
export const SITE_DESCRIPTION =
  "Raspberry Pi 4 software that takes the live feed from DJI FPV goggles or a USB camera and sends it to HDMI, any browser, RTSP/SRT/RTMP/UDP streams and recordings.";

export const SITE_URL = "https://ystreamer.yarosfpv.com";
export const GITHUB_URL = "https://github.com/YLabs-FPV/YStreamer";
export const RELEASES_URL = `${GITHUB_URL}/releases/latest`;
export const YOUTUBE_URL = "https://youtube.com/@yarosfpv";

export const INSTALL_COMMAND = `curl -fsSL ${SITE_URL}/install.sh | sudo sh`;

const LATEST_DOWNLOAD = `${GITHUB_URL}/releases/latest/download`;
export const INSTALL_SCRIPT_TARGET = `${LATEST_DOWNLOAD}/install.sh`;
export const UPDATE_MANIFEST_TARGET = `${LATEST_DOWNLOAD}/manifest.json`;

// Starts the download of the latest SD card image, ystreamer-<version>.img.xz.
// The endpoint behind it (src/pages/download/ystreamer.img.xz.ts) finds the
// file next to the .deb in the release the latest manifest points at.
export const IMAGE_DOWNLOAD = "/download/ystreamer.img.xz";

// The home page video, one per theme; the page plays the one that matches.
// Each poster is the video's first frame, shown until it starts playing
export const HERO_VIDEO = {
  light: "/videos/light/hero.mp4",
  dark: "/videos/dark/hero.mp4",
  lightPoster: "/videos/light/hero-poster.webp",
  darkPoster: "/videos/dark/hero-poster.webp",
};

export type TestedResult =
  "worksFull" | "worksLimited" | "worksPartial" | "doesntWork";
export type TestedBy = "maintainer" | "community";

export interface TestedSetup {
  device: string;
  airUnits?: string[];
  piRevision?: string;
  result: TestedResult;
  by: TestedBy;
  notes?: string;
}

export const TESTED_HARDWARE: TestedSetup[] = [
  {
    device: "DJI Goggles Integra",
    airUnits: ["O3 Air Unit", "O4 Lite", "O4  Pro"],
    piRevision: "1.5",
    result: "worksFull",
    by: "maintainer",
  },
  {
    device: "Eachine ROTG02 Analog Receiver",
    piRevision: "1.5",
    result: "worksFull",
    by: "maintainer",
  },
  {
    device: "DJI Action 4 (Webcam mode)",
    piRevision: "1.5",
    result: "worksLimited",
    by: "maintainer",
  },
];
