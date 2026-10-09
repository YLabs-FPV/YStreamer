import {
  NAL_AUD,
  NAL_IDR,
  NAL_PPS,
  NAL_SPS,
  TIMESCALE,
  describeTrack,
  fragment,
  initSegment,
  nalType,
  splitNals,
  type VideoTrack,
} from "@/lib/fmp4";

// Buffer the player aims to hold. Stalls add to it and the catch-up drains
// it back, so it settles about as high as the link's jitter needs
const TARGET_BUFFER = 0.05;
// Past this the player is too far behind to catch up by speeding, so jump
const MAX_BUFFER = 1.5;
// Playback speeds up in proportion to the excess, so latency a stall added
// drains away instead of staying until a reload
const CATCH_UP_GAIN = 0.5;
const MAX_RATE = 1.1;
// Frames waiting on a busy SourceBuffer, e.g. while the tab is hidden
const MAX_QUEUED = 120;
const KEEP_BEHIND = 10;
// Frame rate is the mean interval over this many recent frames
const RATE_WINDOW = 60;
// A longer gap is a stall, not a frame interval: the link was degrading or
// the splash was up, and the rate before it says nothing about the rate after
const STALL_MICROS = 200_000;

export interface SocketStats {
  width: number;
  height: number;
  fps: number;
  bitsPerSecond: number;
  buffered: number;
}

export interface SocketStreamEvents {
  onplaying: () => void;
  onclose: (busy: boolean) => void;
}

const MediaSourceImpl: typeof MediaSource | undefined =
  (window as any).ManagedMediaSource ?? window.MediaSource;

export function socketStreamSupported() {
  return !!MediaSourceImpl;
}

// Receives access units over /video and plays them through Media Source
// Extensions. The Pi only frames bytes onto TCP; muxing happens here
export class SocketStream {
  #video: HTMLVideoElement;
  #events: SocketStreamEvents;
  #ms!: MediaSource;
  #url = "";
  #ws: WebSocket | null = null;
  #sb: SourceBuffer | null = null;
  #track: VideoTrack | null = null;
  #queue: Uint8Array[] = [];
  #needKeyframe = true;
  #closed = false;

  #sequence = 1;
  #decodeTime = 0;
  #arrivals: number[] = [];
  #frameMicros = 1e6 / 60;

  #bytes = 0;
  #bytesAt = performance.now();
  #bitsPerSecond = 0;

  constructor(video: HTMLVideoElement, events: SocketStreamEvents) {
    this.#video = video;
    this.#events = events;
    // ManagedMediaSource (Safari) refuses to play without this
    video.disableRemotePlayback = true;
    video.srcObject = null;
    video.addEventListener("playing", this.#onPlaying);
    this.#openMedia(() => this.#connect());
  }

  stats(): SocketStats {
    const now = performance.now();
    if (now - this.#bytesAt >= 1000) {
      this.#bitsPerSecond = (this.#bytes * 8 * 1000) / (now - this.#bytesAt);
      this.#bytes = 0;
      this.#bytesAt = now;
    }
    return {
      width: this.#track?.width ?? 0,
      height: this.#track?.height ?? 0,
      fps: 1e6 / this.#frameMicros,
      bitsPerSecond: this.#bitsPerSecond,
      buffered: this.#ahead(),
    };
  }

  close() {
    if (this.#closed) return;
    this.#closed = true;
    this.#video.removeEventListener("playing", this.#onPlaying);
    if (this.#ws) {
      this.#ws.onclose = null;
      this.#ws.onmessage = null;
      this.#ws.close();
    }
    this.#video.removeAttribute("src");
    this.#video.load();
    URL.revokeObjectURL(this.#url);
  }

  #onPlaying = () => this.#events.onplaying();

  #openMedia(onopen: () => void) {
    if (this.#url) URL.revokeObjectURL(this.#url);
    const ms = new MediaSourceImpl!();
    this.#ms = ms;
    this.#sb = null;
    this.#url = URL.createObjectURL(ms);
    this.#video.src = this.#url;
    ms.addEventListener(
      "sourceopen",
      () => {
        if (ms === this.#ms && !this.#closed) onopen();
      },
      { once: true },
    );
  }

  #connect() {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${proto}://${location.host}/api/video`);
    ws.binaryType = "arraybuffer";
    ws.onmessage = (ev) => this.#receive(ev.data as ArrayBuffer);
    ws.onclose = (ev) => {
      if (this.#closed) return;
      this.close();
      this.#events.onclose(ev.code === 1013);
    };
    this.#ws = ws;
  }

  #fail() {
    if (this.#closed) return;
    this.close();
    this.#events.onclose(false);
  }

  #receive(data: ArrayBuffer) {
    if (this.#closed || data.byteLength <= 8) return;
    this.#bytes += data.byteLength;
    const timestamp = Number(new DataView(data).getBigUint64(0));
    const nals = splitNals(new Uint8Array(data, 8));

    this.#measureFrameRate(timestamp);

    let sps: Uint8Array | undefined;
    let pps: Uint8Array | undefined;
    let keyframe = false;
    const samples: Uint8Array[] = [];
    for (const nal of nals) {
      const type = nalType(nal);
      if (type === NAL_SPS) sps = nal;
      else if (type === NAL_PPS) pps = nal;
      else if (type !== NAL_AUD) samples.push(nal);
      if (type === NAL_IDR) keyframe = true;
    }

    if (this.#queue.length > MAX_QUEUED) this.#restartAtKeyframe();
    if (this.#needKeyframe && !keyframe) return;

    if (sps && pps && !sameBytes(sps, this.#track?.sps)) {
      if (!this.#setTrack(describeTrack(sps, pps))) return;
    }
    if (!this.#track || !samples.length) return;
    this.#needKeyframe = false;

    const duration = Math.round((this.#frameMicros * TIMESCALE) / 1e6);
    this.#queue.push(
      fragment(this.#sequence++, samples, keyframe, this.#decodeTime, duration),
    );
    this.#decodeTime += duration;
    this.#pump();
  }

  // Frames are laid out on an evenly spaced timeline at the measured rate
  // rather than at their arrival times, so jitter on the way doesn't reach
  // the screen; the buffer absorbs it instead
  #measureFrameRate(timestamp: number) {
    const arrivals = this.#arrivals;
    const last = arrivals[arrivals.length - 1];
    if (last !== undefined && timestamp - last >= STALL_MICROS) {
      arrivals.length = 0;
    }
    arrivals.push(timestamp);
    if (arrivals.length > RATE_WINDOW + 1) arrivals.shift();
    const span = timestamp - arrivals[0];
    if (arrivals.length > 2 && span > 0) {
      this.#frameMicros = span / (arrivals.length - 1);
    }
  }

  #setTrack(track: VideoTrack): boolean {
    const mime = `video/mp4; codecs="${track.codec}"`;
    if (!MediaSourceImpl!.isTypeSupported(mime)) {
      console.error(`[video socket] browser can't play ${mime}`);
      this.#fail();
      return false;
    }
    const first = !this.#track;
    this.#track = track;
    this.#queue = [initSegment(track)];
    this.#sequence = 1;
    this.#decodeTime = 0;
    if (first) return this.#addSourceBuffer();
    // A fresh MediaSource per track, as a page reload gets. Switching in
    // place with changeType left latency growing after a splash until reload
    this.#openMedia(() => {
      if (this.#addSourceBuffer()) this.#pump();
    });
    return true;
  }

  #addSourceBuffer(): boolean {
    try {
      const sb = this.#ms.addSourceBuffer(
        `video/mp4; codecs="${this.#track!.codec}"`,
      );
      sb.addEventListener("updateend", () => this.#pump());
      sb.addEventListener("error", () => {
        if (sb === this.#sb) this.#fail();
      });
      this.#sb = sb;
      return true;
    } catch (e) {
      console.error("[video socket]", e);
      this.#fail();
      return false;
    }
  }

  #pump() {
    const sb = this.#sb;
    if (this.#closed || !sb || sb.updating) return;
    this.#chase();

    const buffered = sb.buffered;
    const now = this.#video.currentTime;
    if (buffered.length && now - buffered.start(0) > KEEP_BEHIND * 2) {
      sb.remove(0, now - KEEP_BEHIND);
      return;
    }

    if (!this.#queue.length) return;
    const chunk = concat(this.#queue);
    this.#queue = [];
    try {
      sb.appendBuffer(chunk as BufferSource);
    } catch (e) {
      if ((e as DOMException).name === "QuotaExceededError") {
        this.#restartAtKeyframe();
        if (buffered.length) sb.remove(0, Math.max(0, now - 1));
      } else {
        console.error("[video socket]", e);
        this.#fail();
      }
    }
  }

  // The dropped frames may have included the init segment, and nothing after
  // them decodes without their reference frames
  #restartAtKeyframe() {
    this.#queue = this.#track ? [initSegment(this.#track)] : [];
    this.#needKeyframe = true;
  }

  #ahead(): number {
    const b = this.#video.buffered;
    if (!b.length) return 0;
    return Math.max(0, b.end(b.length - 1) - this.#video.currentTime);
  }

  #chase() {
    const video = this.#video;
    const b = video.buffered;
    if (!b.length) return;
    const start = b.start(b.length - 1);
    const end = b.end(b.length - 1);
    const ahead = end - video.currentTime;

    if (ahead > MAX_BUFFER || video.currentTime < start) {
      video.currentTime = Math.max(start, end - TARGET_BUFFER);
    }
    const excess = Math.max(0, ahead - TARGET_BUFFER);
    const rate = Math.min(MAX_RATE, 1 + excess * CATCH_UP_GAIN);
    const rounded = Math.round(rate * 100) / 100;
    if (video.playbackRate !== rounded) video.playbackRate = rounded;

    if (video.paused && ahead >= TARGET_BUFFER) {
      // A burst on arrival would otherwise stay as latency for good
      if (ahead > TARGET_BUFFER * 1.5) {
        video.currentTime = Math.max(start, end - TARGET_BUFFER);
      }
      video.play().catch(() => {});
    }
  }
}

function sameBytes(a: Uint8Array, b: Uint8Array | undefined) {
  return !!b && a.length === b.length && a.every((v, i) => v === b[i]);
}

function concat(parts: Uint8Array[]): Uint8Array {
  if (parts.length === 1) return parts[0];
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}
