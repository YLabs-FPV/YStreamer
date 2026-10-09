// Minimal fragmented-MP4 muxer for a single H.264 track

export const NAL_IDR = 5;
export const NAL_SPS = 7;
export const NAL_PPS = 8;
export const NAL_AUD = 9;

export const TIMESCALE = 90_000;

export function splitNals(au: Uint8Array): Uint8Array[] {
  const nals: Uint8Array[] = [];
  let start = -1;
  let i = 0;
  while (i + 3 <= au.length) {
    if (au[i] === 0 && au[i + 1] === 0 && au[i + 2] === 1) {
      if (start >= 0) nals.push(trimZeros(au.subarray(start, i)));
      i += 3;
      start = i;
    } else {
      i++;
    }
  }
  if (start >= 0 && start < au.length) nals.push(au.subarray(start));
  return nals.filter((n) => n.length > 0);
}

// A four-byte start code leaves its leading zero on the previous NAL
function trimZeros(nal: Uint8Array): Uint8Array {
  let end = nal.length;
  while (end > 0 && nal[end - 1] === 0) end--;
  return nal.subarray(0, end);
}

export const nalType = (nal: Uint8Array) => nal[0] & 0x1f;

export interface VideoTrack {
  sps: Uint8Array;
  pps: Uint8Array;
  width: number;
  height: number;
  codec: string;
}

export function describeTrack(sps: Uint8Array, pps: Uint8Array): VideoTrack {
  const hex = (b: number) => b.toString(16).padStart(2, "0");
  const { width, height } = spsDimensions(sps);
  return {
    sps,
    pps,
    width,
    height,
    codec: `avc1.${hex(sps[1])}${hex(sps[2])}${hex(sps[3])}`,
  };
}

class BitReader {
  #bytes: Uint8Array;
  #pos = 0;

  constructor(bytes: Uint8Array) {
    this.#bytes = bytes;
  }

  bit(): number {
    const byte = this.#bytes[this.#pos >> 3] ?? 0;
    const bit = (byte >> (7 - (this.#pos & 7))) & 1;
    this.#pos++;
    return bit;
  }

  bits(n: number): number {
    let v = 0;
    for (let i = 0; i < n; i++) v = v * 2 + this.bit();
    return v;
  }

  ue(): number {
    let zeros = 0;
    while (this.bit() === 0 && zeros < 32) zeros++;
    return 2 ** zeros - 1 + this.bits(zeros);
  }

  se(): number {
    const v = this.ue();
    return v & 1 ? (v + 1) / 2 : -v / 2;
  }
}

function unescapeRbsp(nal: Uint8Array): Uint8Array {
  const out = new Uint8Array(nal.length);
  let n = 0;
  for (let i = 0; i < nal.length; i++) {
    if (i >= 2 && nal[i] === 3 && nal[i - 1] === 0 && nal[i - 2] === 0) {
      continue;
    }
    out[n++] = nal[i];
  }
  return out.subarray(0, n);
}

function spsDimensions(sps: Uint8Array) {
  const r = new BitReader(unescapeRbsp(sps.subarray(1)));
  const profile = r.bits(8);
  r.bits(16); // constraint flags, level
  r.ue(); // seq_parameter_set_id
  let chromaFormat = 1;
  if (
    [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134].includes(profile)
  ) {
    chromaFormat = r.ue();
    if (chromaFormat === 3) r.bit();
    r.ue(); // bit_depth_luma
    r.ue(); // bit_depth_chroma
    r.bit(); // qpprime_y_zero_transform_bypass
    if (r.bit()) {
      const lists = chromaFormat === 3 ? 12 : 8;
      for (let i = 0; i < lists; i++) {
        if (!r.bit()) continue;
        const size = i < 6 ? 16 : 64;
        let last = 8;
        let next = 8;
        for (let j = 0; j < size; j++) {
          if (next !== 0) next = (last + r.se() + 256) % 256;
          last = next === 0 ? last : next;
        }
      }
    }
  }
  r.ue(); // log2_max_frame_num
  const pocType = r.ue();
  if (pocType === 0) {
    r.ue();
  } else if (pocType === 1) {
    r.bit();
    r.se();
    r.se();
    const cycle = r.ue();
    for (let i = 0; i < cycle; i++) r.se();
  }
  r.ue(); // max_num_ref_frames
  r.bit(); // gaps_in_frame_num_allowed
  const widthMbs = r.ue() + 1;
  const heightMapUnits = r.ue() + 1;
  const frameMbsOnly = r.bit();
  if (!frameMbsOnly) r.bit();
  r.bit(); // direct_8x8_inference
  let [left, right, top, bottom] = [0, 0, 0, 0];
  if (r.bit()) {
    left = r.ue();
    right = r.ue();
    top = r.ue();
    bottom = r.ue();
  }
  const cropX = chromaFormat === 0 || chromaFormat === 3 ? 1 : 2;
  const cropY = (chromaFormat === 1 ? 2 : 1) * (2 - frameMbsOnly);
  return {
    width: widthMbs * 16 - (left + right) * cropX,
    height: (2 - frameMbsOnly) * heightMapUnits * 16 - (top + bottom) * cropY,
  };
}

function box(type: string, ...payload: Uint8Array[]): Uint8Array {
  const size = 8 + payload.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  view.setUint32(0, size);
  for (let i = 0; i < 4; i++) out[4 + i] = type.charCodeAt(i);
  let offset = 8;
  for (const p of payload) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}

function fullBox(
  type: string,
  version: number,
  flags: number,
  ...payload: Uint8Array[]
) {
  return box(type, u32((version << 24) | flags), ...payload);
}

function u8(...values: number[]) {
  return new Uint8Array(values);
}

function u16(v: number) {
  return u8((v >> 8) & 0xff, v & 0xff);
}

function u32(v: number) {
  const out = new Uint8Array(4);
  new DataView(out.buffer).setUint32(0, v >>> 0);
  return out;
}

function u64(v: number) {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, BigInt(Math.floor(v)));
  return out;
}

const zeros = (n: number) => new Uint8Array(n);

const MATRIX = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000];
const matrix = () => concat(...MATRIX.map(u32));

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}

export function initSegment(t: VideoTrack): Uint8Array {
  const ftyp = box(
    "ftyp",
    new TextEncoder().encode("iso5"),
    u32(512),
    new TextEncoder().encode("iso5iso6mp41"),
  );

  const mvhd = fullBox(
    "mvhd",
    0,
    0,
    u32(0),
    u32(0),
    u32(1000),
    u32(0),
    u32(0x10000),
    u16(0x100),
    zeros(10),
    matrix(),
    zeros(24),
    u32(2),
  );

  const tkhd = fullBox(
    "tkhd",
    0,
    3,
    u32(0),
    u32(0),
    u32(1),
    u32(0),
    u32(0),
    zeros(8),
    u16(0),
    u16(0),
    u16(0),
    u16(0),
    matrix(),
    u32(t.width << 16),
    u32(t.height << 16),
  );

  const mdhd = fullBox(
    "mdhd",
    0,
    0,
    u32(0),
    u32(0),
    u32(TIMESCALE),
    u32(0),
    u16(0x55c4),
    u16(0),
  );
  const hdlr = fullBox(
    "hdlr",
    0,
    0,
    u32(0),
    new TextEncoder().encode("vide"),
    zeros(12),
    new TextEncoder().encode("VideoHandler\0"),
  );

  const avcC = box(
    "avcC",
    u8(1, t.sps[1], t.sps[2], t.sps[3], 0xff, 0xe1),
    u16(t.sps.length),
    t.sps,
    u8(1),
    u16(t.pps.length),
    t.pps,
  );
  const avc1 = box(
    "avc1",
    zeros(6),
    u16(1),
    zeros(16),
    u16(t.width),
    u16(t.height),
    u32(0x480000),
    u32(0x480000),
    u32(0),
    u16(1),
    zeros(32),
    u16(0x18),
    u16(0xffff),
    avcC,
  );
  const stbl = box(
    "stbl",
    fullBox("stsd", 0, 0, u32(1), avc1),
    fullBox("stts", 0, 0, u32(0)),
    fullBox("stsc", 0, 0, u32(0)),
    fullBox("stsz", 0, 0, u32(0), u32(0)),
    fullBox("stco", 0, 0, u32(0)),
  );
  const minf = box(
    "minf",
    fullBox("vmhd", 0, 1, zeros(8)),
    box("dinf", fullBox("dref", 0, 0, u32(1), fullBox("url ", 0, 1))),
    stbl,
  );
  const trak = box("trak", tkhd, box("mdia", mdhd, hdlr, minf));
  const mvex = box(
    "mvex",
    fullBox("trex", 0, 0, u32(1), u32(1), u32(0), u32(0), u32(0)),
  );

  return concat(ftyp, box("moov", mvhd, trak, mvex));
}

// Times are in TIMESCALE ticks. The goggles send no B-frames, so presentation
// time equals decode time and no composition offsets are written
export function fragment(
  sequence: number,
  nals: Uint8Array[],
  keyframe: boolean,
  decodeTime: number,
  duration: number,
): Uint8Array {
  const size = nals.reduce((n, nal) => n + 4 + nal.length, 0);
  const sampleFlags = keyframe ? 0x02000000 : 0x01010000;

  const build = (dataOffset: number) =>
    box(
      "moof",
      fullBox("mfhd", 0, 0, u32(sequence)),
      box(
        "traf",
        fullBox("tfhd", 0, 0x020000, u32(1)),
        fullBox("tfdt", 1, 0, u64(decodeTime)),
        fullBox(
          "trun",
          0,
          0x000701,
          u32(1),
          u32(dataOffset),
          u32(duration),
          u32(size),
          u32(sampleFlags),
        ),
      ),
    );
  const moofSize = build(0).length;
  const moof = build(moofSize + 8);

  const mdat = new Uint8Array(8 + size);
  const view = new DataView(mdat.buffer);
  view.setUint32(0, mdat.length);
  mdat.set(new TextEncoder().encode("mdat"), 4);
  let offset = 8;
  for (const nal of nals) {
    view.setUint32(offset, nal.length);
    mdat.set(nal, offset + 4);
    offset += 4 + nal.length;
  }
  return concat(moof, mdat);
}
