import type { Status } from "@/types/stream";

export function statusLabel(status: Status): string {
  return status === "playing"
    ? "Live"
    : status === "connected"
      ? "Waiting for video"
      : status === "busy"
        ? "Viewer limit reached"
        : status === "disconnected"
          ? "Reconnecting"
          : status === "error"
            ? "Connection failed"
            : "Connecting";
}

export function statusDot(status: Status): string {
  return status === "playing"
    ? "bg-ok"
    : status === "disconnected" || status === "error"
      ? "bg-down"
      : "bg-warn";
}
