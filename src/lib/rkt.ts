// Free layouts for the Custom tool, and the .rkt file that stores them.
//
// A layout is a strip of paper `width` mm wide holding positioned items.
// Coordinates are millimetres from the top left corner of the printable
// area; each item is a box (x, y, w, h) rotated by `rotate` degrees around
// its centre. On a printer with another width the whole layout is scaled.
import { fontCss, type Align, type FontId } from "./canvas";

interface Box {
  id: number;
  x: number;
  y: number;
  w: number;
  h: number;
  rotate: number;
}

export interface TextItem extends Box {
  type: "text";
  text: string;
  font: FontId;
  /** Font size in mm. */
  size: number;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  align: Align;
  /** White text on a black box. */
  invert: boolean;
}

export interface ImageItem extends Box {
  type: "image";
  /** Key in `Layout.images`. */
  image: string;
  brightness: number;
  contrast: number;
  invert: boolean;
  flip: boolean;
}

export interface ShapeItem extends Box {
  type: "line" | "rect" | "ellipse";
  /** Stroke width in mm. */
  stroke: number;
  dashed: boolean;
  fill: boolean;
  /** Corner radius of rectangles, in mm. */
  radius: number;
}

export type Item = TextItem | ImageItem | ShapeItem;
export type ItemType = Item["type"];

export interface Layout {
  /** Printable width in mm the layout was made for. */
  width: number;
  /** Paper length in mm, or null to end after the lowest item. */
  length: number | null;
  items: Item[];
  /** Pictures as data URLs, shared by the items that use them. */
  images: Record<string, string>;
}

export const PT = 0.3528; // mm per typographic point
const MARGIN = 3; // mm left under the lowest item when the length is automatic
export const MIN_LENGTH = 10;

let lastId = 0;
export const newId = () => ++lastId;

export function emptyLayout(width: number): Layout {
  return { width, length: null, items: [], images: {} };
}

/** A new item of the given type, centred around (cx, cy). */
export function makeItem(type: ItemType, cx: number, cy: number, width: number, extra: Partial<Item> = {}): Item {
  const base = { id: newId(), rotate: 0 };
  let item: Item;
  if (type === "text") {
    const w = Math.min(width - 4, 40);
    item = { ...base, type, x: cx - w / 2, y: cy - 4, w, h: 8, text: "", font: "sans", size: 11 * PT, bold: false, italic: false, underline: false, align: "center", invert: false };
  } else if (type === "image") {
    item = { ...base, type, x: cx - 15, y: cy - 15, w: 30, h: 30, image: "", brightness: 0, contrast: 0, invert: false, flip: false };
  } else {
    const shape = { stroke: 0.6, dashed: false, fill: false, radius: 0 };
    if (type === "line") {
      const w = width * 0.8;
      item = { ...base, ...shape, type, x: cx - w / 2, y: cy - 0.3, w, h: 0.6 };
    } else {
      const s = Math.min(24, width * 0.5);
      item = { ...base, ...shape, type, x: cx - s / 2, y: cy - s / 2, w: s, h: s };
    }
  }
  return { ...item, ...extra } as Item;
}

/** The four corners of an item after rotation. */
export function corners(it: Box): [number, number][] {
  const cx = it.x + it.w / 2;
  const cy = it.y + it.h / 2;
  const a = (it.rotate * Math.PI) / 180;
  const cos = Math.cos(a);
  const sin = Math.sin(a);
  return [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ].map(([sx, sy]) => {
    const dx = (sx * it.w) / 2;
    const dy = (sy * it.h) / 2;
    return [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos];
  });
}

export function bounds(it: Box) {
  const c = corners(it);
  const xs = c.map((p) => p[0]);
  const ys = c.map((p) => p[1]);
  return { left: Math.min(...xs), right: Math.max(...xs), top: Math.min(...ys), bottom: Math.max(...ys) };
}

/** Paper length in mm: fixed, or just below the lowest item. */
export function paperLength(l: Layout): number {
  if (l.length) return l.length;
  const bottom = l.items.reduce((m, it) => Math.max(m, bounds(it).bottom), 0);
  return Math.max(MIN_LENGTH, Math.ceil(bottom + MARGIN));
}

function blank(w: number, h: number) {
  const c = document.createElement("canvas");
  c.width = Math.max(1, Math.round(w));
  c.height = Math.max(1, Math.round(h));
  return c;
}

export const textFont = (it: TextItem, px: number) => `${it.italic ? "italic " : ""}${it.bold ? 700 : 400} ${px}px ${fontCss(it.font)}`;
export const LINE = 1.22;
export const PAD = 0.6; // mm of padding inside text boxes

function wrapLines(ctx: CanvasRenderingContext2D, text: string, max: number): string[] {
  const out: string[] = [];
  for (const para of text.split("\n")) {
    let line = "";
    for (const word of para.split(/(\s+)/)) {
      if (!word) continue;
      const next = line + word;
      if (ctx.measureText(next).width <= max || !line.trim()) {
        if (ctx.measureText(next).width > max && !line.trim()) {
          // A word wider than the box is split by characters.
          let chunk = "";
          for (const ch of next) {
            if (ctx.measureText(chunk + ch).width > max && chunk) {
              out.push(chunk);
              chunk = "";
            }
            chunk += ch;
          }
          line = chunk;
        } else line = next;
      } else {
        out.push(line.trimEnd());
        line = word.trimStart();
      }
    }
    out.push(line.trimEnd());
  }
  return out;
}

const probe = () => blank(1, 1).getContext("2d")!;

/** Height in mm that a text item needs at its width. */
export function textHeight(it: TextItem): number {
  // Measured at a fixed scale so the height does not depend on the printer.
  const s = 8;
  const ctx = probe();
  ctx.font = textFont(it, it.size * s);
  const lines = wrapLines(ctx, it.text || " ", Math.max(1, (it.w - PAD * 2) * s));
  return lines.length * it.size * LINE + PAD * 2;
}

/** Draws one item, unrotated, at `s` pixels per mm, on a transparent canvas. */
export function renderItem(it: Item, s: number, images: Map<string, HTMLImageElement>): HTMLCanvasElement {
  const c = blank(it.w * s, it.h * s);
  const ctx = c.getContext("2d")!;
  const W = c.width;
  const H = c.height;
  ctx.fillStyle = "#000";
  ctx.strokeStyle = "#000";
  if (it.type === "text") {
    if (it.invert) ctx.fillRect(0, 0, W, H);
    const px = it.size * s;
    ctx.font = textFont(it, px);
    ctx.textBaseline = "alphabetic";
    ctx.fillStyle = it.invert ? "#fff" : "#000";
    ctx.strokeStyle = ctx.fillStyle;
    const pad = PAD * s;
    const lh = px * LINE;
    const lines = wrapLines(ctx, it.text || "", Math.max(1, W - pad * 2));
    lines.forEach((l, i) => {
      const w = ctx.measureText(l).width;
      const x = it.align === "left" ? pad : it.align === "right" ? W - pad - w : (W - w) / 2;
      const y = pad + i * lh + px * 0.95;
      ctx.fillText(l, x, y);
      if (it.underline && l) {
        ctx.lineWidth = Math.max(1, px * 0.07);
        ctx.beginPath();
        ctx.moveTo(x, y + px * 0.12);
        ctx.lineTo(x + w, y + px * 0.12);
        ctx.stroke();
      }
    });
  } else if (it.type === "image") {
    const img = images.get(it.image);
    if (img) {
      const f: string[] = [];
      if (it.brightness) f.push(`brightness(${100 + it.brightness}%)`);
      if (it.contrast) f.push(`contrast(${100 + it.contrast}%)`);
      if (it.invert) f.push("invert(1)");
      ctx.filter = f.join(" ") || "none";
      if (it.flip) {
        ctx.translate(W, 0);
        ctx.scale(-1, 1);
      }
      ctx.drawImage(img, 0, 0, W, H);
    }
  } else {
    const lw = Math.max(1, it.stroke * s);
    ctx.lineWidth = lw;
    if (it.dashed) ctx.setLineDash([Math.max(3, lw * 3), Math.max(2, lw * 2)]);
    ctx.beginPath();
    if (it.type === "line") {
      ctx.moveTo(0, H / 2);
      ctx.lineTo(W, H / 2);
      ctx.lineWidth = Math.max(1, H);
      if (it.dashed) ctx.setLineDash([Math.max(3, H * 3), Math.max(2, H * 2)]);
      ctx.stroke();
    } else if (it.type === "rect") {
      const r = Math.min(it.radius * s, W / 2, H / 2);
      if (it.fill) ctx.roundRect(0, 0, W, H, r);
      else ctx.roundRect(lw / 2, lw / 2, W - lw, H - lw, Math.max(0, r - lw / 2));
      if (it.fill) ctx.fill();
      else ctx.stroke();
    } else {
      const inset = it.fill ? 0 : lw / 2;
      ctx.ellipse(W / 2, H / 2, Math.max(0.5, W / 2 - inset), Math.max(0.5, H / 2 - inset), 0, 0, Math.PI * 2);
      if (it.fill) ctx.fill();
      else ctx.stroke();
    }
  }
  return c;
}

/** The whole layout as one page, `widthPx` wide. */
export function renderLayout(l: Layout, widthPx: number, images: Map<string, HTMLImageElement>): HTMLCanvasElement {
  const s = widthPx / l.width;
  const c = blank(widthPx, paperLength(l) * s);
  const ctx = c.getContext("2d")!;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, c.width, c.height);
  for (const it of l.items) {
    const part = renderItem(it, s, images);
    ctx.save();
    ctx.translate((it.x + it.w / 2) * s, (it.y + it.h / 2) * s);
    ctx.rotate((it.rotate * Math.PI) / 180);
    ctx.drawImage(part, (-it.w * s) / 2, (-it.h * s) / 2, it.w * s, it.h * s);
    ctx.restore();
  }
  return c;
}

// .rkt files: JSON with a format marker, readable and easy to generate.

export const RKT_FORMAT = "raketo-layout";
export const RKT_VERSION = 1;

export function serialize(l: Layout): string {
  const used = new Set(l.items.flatMap((i) => (i.type === "image" ? [i.image] : [])));
  const images = Object.fromEntries(Object.entries(l.images).filter(([k]) => used.has(k)));
  const items = l.items.map(({ id: _id, ...rest }) => rest);
  return JSON.stringify({ format: RKT_FORMAT, version: RKT_VERSION, width: l.width, length: l.length, items, images }, null, 1);
}

const num = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) ? v : d);
const bool = (v: unknown) => v === true;
const str = (v: unknown, d = "") => (typeof v === "string" ? v : d);
const oneOf = <T extends string>(v: unknown, all: readonly T[], d: T): T => (all.includes(v as T) ? (v as T) : d);

/** Reads a .rkt file; unknown fields are ignored and missing ones take defaults. */
export function parse(text: string): Layout {
  const raw = JSON.parse(text);
  if (!raw || raw.format !== RKT_FORMAT) throw new Error("not a Raketo layout");
  if (num(raw.version, 1) > RKT_VERSION) throw new Error("made by a newer Raketo");
  const width = Math.max(10, num(raw.width, 48));
  const images: Record<string, string> = {};
  for (const [k, v] of Object.entries(raw.images ?? {})) if (typeof v === "string" && v.startsWith("data:image/")) images[k] = v;
  const items: Item[] = [];
  for (const r of Array.isArray(raw.items) ? raw.items : []) {
    const type = oneOf(r?.type, ["text", "image", "line", "rect", "ellipse"] as const, "text");
    const base = makeItem(type, 0, 0, width);
    const box = { x: num(r.x, 0), y: num(r.y, 0), w: Math.max(0.2, num(r.w, base.w)), h: Math.max(0.2, num(r.h, base.h)), rotate: num(r.rotate, 0) };
    if (base.type === "text") {
      items.push({
        ...base,
        ...box,
        text: str(r.text),
        font: oneOf(r.font, ["sans", "serif", "mono", "round"] as const, "sans"),
        size: Math.max(0.5, num(r.size, base.size)),
        bold: bool(r.bold),
        italic: bool(r.italic),
        underline: bool(r.underline),
        align: oneOf(r.align, ["left", "center", "right"] as const, "center"),
        invert: bool(r.invert),
      });
      // Text boxes always take the height their text needs.
      const last = items[items.length - 1] as TextItem;
      last.h = textHeight(last);
    } else if (base.type === "image") {
      const image = str(r.image);
      if (!images[image]) continue;
      items.push({ ...base, ...box, image, brightness: num(r.brightness, 0), contrast: num(r.contrast, 0), invert: bool(r.invert), flip: bool(r.flip) });
    } else {
      items.push({ ...base, ...box, stroke: Math.max(0.1, num(r.stroke, base.stroke)), dashed: bool(r.dashed), fill: bool(r.fill), radius: Math.max(0, num(r.radius, 0)) });
    }
  }
  const length = raw.length == null ? null : Math.max(MIN_LENGTH, num(raw.length, MIN_LENGTH));
  return { width, length, items, images };
}
