import type { Component } from "svelte";
import type { Section, Settings } from "@/types/settings";
import Video from "@lucide/svelte/icons/video";
import Wifi from "@lucide/svelte/icons/wifi";
import Radio from "@lucide/svelte/icons/radio";
import Cpu from "@lucide/svelte/icons/cpu";
import Monitor from "@lucide/svelte/icons/monitor";
import Circle from "@lucide/svelte/icons/circle-dot";
import Cable from "@lucide/svelte/icons/cable";
import Activity from "@lucide/svelte/icons/activity";
import ScrollText from "@lucide/svelte/icons/scroll-text";
import SquareTerminal from "@lucide/svelte/icons/square-terminal";

export interface NavTab {
  id: string;
  label: string;
  sections: Section[];
  on?: (saved: Settings) => boolean;
  developer?: boolean;
}

export interface NavPage {
  id: string;
  label: string;
  sections: Section[];
  icon: Component;
  tabs?: NavTab[];
  developer?: boolean;
}

export const WATCH: NavPage = {
  id: "watch",
  label: "Watch",
  sections: [],
  icon: Video,
};

export const SETTINGS_PAGES: NavPage[] = [
  {
    id: "input",
    label: "Input",
    sections: ["input"],
    icon: Cable,
  },
  {
    id: "network",
    label: "Network",
    sections: ["wifi", "ethernet"],
    icon: Wifi,
    tabs: [
      {
        id: "connectivity",
        label: "Connectivity",
        sections: ["wifi", "ethernet"],
      },
      { id: "remote", label: "Remote access", sections: [] },
    ],
  },
  {
    id: "streaming",
    label: "Streaming",
    sections: ["rtsp", "srt", "rtmp", "udp", "viewer"],
    icon: Radio,
    tabs: [
      {
        id: "rtsp",
        label: "RTSP",
        sections: ["rtsp"],
        on: (s) => s.rtsp.enabled,
      },
      { id: "srt", label: "SRT", sections: ["srt"], on: (s) => s.srt.enabled },
      {
        id: "rtmp",
        label: "RTMP",
        sections: ["rtmp"],
        on: (s) => s.rtmp.enabled,
      },
      { id: "udp", label: "UDP", sections: ["udp"], on: (s) => s.udp.enabled },
      { id: "browser", label: "Browser", sections: ["viewer"] },
    ],
  },
  {
    id: "recording",
    label: "Recording",
    sections: ["recording"],
    icon: Circle,
    tabs: [
      { id: "files", label: "Files", sections: [] },
      { id: "settings", label: "Settings", sections: ["recording"] },
    ],
  },
  {
    id: "display",
    label: "Display",
    sections: ["display", "splash", "overlay"],
    icon: Monitor,
    tabs: [
      { id: "hdmi", label: "HDMI", sections: ["display"] },
      { id: "splash", label: "Splash screen", sections: ["splash"] },
      { id: "logo", label: "Logo", sections: ["overlay"] },
    ],
  },
  {
    id: "performance",
    label: "Performance",
    sections: [],
    icon: Activity,
  },
  {
    id: "logs",
    label: "Logs",
    sections: [],
    icon: ScrollText,
  },
  {
    id: "system",
    label: "System",
    sections: ["device", "advanced"],
    icon: Cpu,
    tabs: [
      { id: "general", label: "General", sections: ["device"] },
      { id: "about", label: "About", sections: [] },
      { id: "updates", label: "Updates", sections: ["advanced"] },
      { id: "service", label: "Service", sections: [] },
      { id: "configuration", label: "Configuration", sections: [] },
      {
        id: "developer",
        label: "Developer",
        sections: ["advanced"],
        developer: true,
      },
    ],
  },
  {
    id: "terminal",
    label: "Terminal",
    sections: [],
    icon: SquareTerminal,
    developer: true,
  },
];

export const NAV_GROUPS: { label: string; pages: NavPage[] }[] = [
  ["Video", ["input", "streaming", "recording", "display"]],
  ["Device", ["network", "system"]],
  ["Diagnostics", ["performance", "logs", "terminal"]],
].map(([label, ids]) => ({
  label: label as string,
  pages: (ids as string[]).map((id) =>
    SETTINGS_PAGES.find((p) => p.id === id)!,
  ),
}));

export function pageById(id: string): NavPage {
  return SETTINGS_PAGES.find((p) => p.id === id) ?? WATCH;
}

export function routeOf(path: string): { page: NavPage; tab: string } {
  const [id, tab = ""] = path.replace(/^\//, "").split("/");
  return { page: pageById(id), tab };
}
