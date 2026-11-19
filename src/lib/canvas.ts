// Canvas helpers shared by the tools. Every tool draws black on white at the
// printer's exact width; the Rust side does dithering and encoding.
import QRCode from "qrcode";
import JsBarcode from "jsbarcode";

export const FONTS = [
  { id: "sans", css: '"Bricolage Grotesque Variable", "Segoe UI", sans-serif', label: "Grotesque" },
  { id: "serif", css: 'Georgia, "Times New Roman", serif', label: "Serif" },
  { id: "mono", css: 'Consolas, "Courier New", monospace', label: "Mono" },
  { id: "round", css: '"Comic Sans MS", "Segoe Print", cursive', label: "Manuscrite" },
] as const;
export type FontId = (typeof FONTS)[number]["id"];
export const fontCss = (id: FontId) => FONTS.find((f) => f.id === id)!.css;

export type Align = "left" | "center" | "right";

export function canvas(w: number, h: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = Math.max(1, Math.round(w));
  c.height = Math.max(1, Math.round(h));
  const ctx = c.getContext("2d")!;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, c.width, c.height);
  ctx.fillStyle = "#000";
  ctx.strokeStyle = "#000";
  return c;
}

export function wrap(ctx: CanvasRenderingContext2D, text: string, max: number): string[] {
  const out: string[] = [];
  for (const para of text.split("\n")) {
    if (para === "") {
      out.push("");
      continue;
    }
    let line = "";
    for (const word of para.split(/(\s+)/)) {
      const next = line + word;
      if (ctx.measureText(next).width <= max || line === "") {
        // A single word wider than the line is split by characters.
        if (ctx.measureText(next).width > max && line === "") {
          let chunk = "";
          for (const ch of word) {
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

export interface TextStyle {
  size: number;
  bold: boolean;
  align: Align;
  font: FontId;
  lineHeight?: number;
}

export function setFont(ctx: CanvasRenderingContext2D, s: TextStyle) {
  ctx.font = `${s.bold ? 700 : 400} ${s.size}px ${fontCss(s.font)}`;
  ctx.textBaseline = "alphabetic";
}

/** Draws wrapped text; returns the canvas with exactly the needed height. */
export function textBlock(text: string, width: number, s: TextStyle, pad = 0): HTMLCanvasElement {
  const probe = canvas(1, 1).getContext("2d")!;
  setFont(probe, s);
  const inner = width - pad * 2;
  const lines = wrap(probe, text || " ", inner);
  const lh = Math.round(s.size * (s.lineHeight ?? 1.25));
  const c = canvas(width, lines.length * lh + Math.round(s.size * 0.25));
  const ctx = c.getContext("2d")!;
  setFont(ctx, s);
  lines.forEach((l, i) => {
    const w = ctx.measureText(l).width;
    const x = s.align === "left" ? pad : s.align === "right" ? width - pad - w : (width - w) / 2;
    ctx.fillText(l, x, (i + 1) * lh - Math.round(s.size * 0.2));
  });
  return c;
}

export function stack(parts: HTMLCanvasElement[], width: number, gap = 0): HTMLCanvasElement {
  const h = parts.reduce((a, p) => a + p.height, 0) + gap * Math.max(0, parts.length - 1);
  const c = canvas(width, Math.max(h, 1));
  const ctx = c.getContext("2d")!;
  let y = 0;
  for (const p of parts) {
    ctx.drawImage(p, Math.round((width - p.width) / 2), y);
    y += p.height + gap;
  }
  return c;
}

export function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((res, rej) => {
    const i = new Image();
    i.onload = () => res(i);
    i.onerror = () => rej(new Error("image"));
    i.src = src;
  });
}

export function fileToDataUrl(f: Blob): Promise<string> {
  return new Promise((res, rej) => {
    const r = new FileReader();
    r.onload = () => res(r.result as string);
    r.onerror = () => rej(r.error);
    r.readAsDataURL(f);
  });
}

export interface ImageAdjust {
  rotate: 0 | 90 | 180 | 270;
  brightness: number; // -100..100
  contrast: number; // -100..100
  invert: boolean;
}

export const noAdjust: ImageAdjust = { rotate: 0, brightness: 0, contrast: 0, invert: false };

/** Scales an image to `width`, applying rotation and tone adjustments. */
export function fitImage(img: CanvasImageSource & { width: number; height: number }, width: number, a: ImageAdjust = noAdjust): HTMLCanvasElement {
  const swap = a.rotate === 90 || a.rotate === 270;
  const sw = swap ? img.height : img.width;
  const sh = swap ? img.width : img.height;
  const scale = width / sw;
  const c = canvas(width, Math.round(sh * scale));
  const ctx = c.getContext("2d")!;
  const f: string[] = [];
  if (a.brightness) f.push(`brightness(${100 + a.brightness}%)`);
  if (a.contrast) f.push(`contrast(${100 + a.contrast}%)`);
  if (a.invert) f.push("invert(1)");
  ctx.filter = f.join(" ") || "none";
  ctx.translate(c.width / 2, c.height / 2);
  ctx.rotate((a.rotate * Math.PI) / 180);
  const dw = img.width * scale;
  const dh = img.height * scale;
  ctx.drawImage(img, -dw / 2, -dh / 2, dw, dh);
  return c;
}

export async function qrCanvas(value: string, size: number): Promise<HTMLCanvasElement> {
  const c = canvas(size, size);
  await QRCode.toCanvas(c, value || " ", { width: size, margin: 1, errorCorrectionLevel: "M", color: { dark: "#000", light: "#fff" } });
  return c;
}

export function barcodeCanvas(value: string, width: number, height: number): HTMLCanvasElement | null {
  const c = document.createElement("canvas");
  try {
    JsBarcode(c, value || "0", { format: "CODE128", displayValue: true, margin: 4, height, fontSize: Math.max(12, height / 4), background: "#fff", lineColor: "#000" });
  } catch {
    return null;
  }
  if (c.width <= width) return c;
  const out = canvas(width, Math.round((c.height * width) / c.width));
  const ctx = out.getContext("2d")!;
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(c, 0, 0, out.width, out.height);
  return out;
}

export function toPng(c: HTMLCanvasElement): string {
  return c.toDataURL("image/png");
}

export function mmToPx(mm: number, dpi: number) {
  return Math.round((mm * dpi) / 25.4);
}
