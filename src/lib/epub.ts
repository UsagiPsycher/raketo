// EPUB books reflowed to the roll width: text blocks and pictures, no page layout.
import { strFromU8, unzipSync } from "fflate";
import { canvas, fitImage, loadImage, noAdjust, textBlock, type FontId } from "./canvas";

export interface Chapter {
  title: string;
  path: string;
  body: Element;
}

export interface Book {
  title: string;
  chapters: Chapter[];
  files: Record<string, Uint8Array>;
}

function resolve(base: string, href: string): string {
  const parts = base.includes("/") ? base.slice(0, base.lastIndexOf("/")).split("/") : [];
  let clean = href.split(/[#?]/)[0];
  try {
    clean = decodeURIComponent(clean);
  } catch {
    // keep as is
  }
  for (const seg of clean.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") parts.pop();
    else parts.push(seg);
  }
  return parts.join("/");
}

const xml = (s: string) => new DOMParser().parseFromString(s, "application/xml");
const byTag = (d: Document | Element, name: string) => Array.from(d.getElementsByTagNameNS("*", name));

export function openEpub(data: Uint8Array): Book {
  const files = unzipSync(data);
  const text = (p: string) => (files[p] ? strFromU8(files[p]) : null);
  const container = text("META-INF/container.xml");
  if (!container) throw new Error("not an EPUB");
  const opfPath = byTag(xml(container), "rootfile")[0]?.getAttribute("full-path");
  const opfText = opfPath ? text(opfPath) : null;
  if (!opfPath || !opfText) throw new Error("not an EPUB");
  const opf = xml(opfText);
  const manifest = new Map<string, string>();
  for (const it of byTag(opf, "item")) {
    const id = it.getAttribute("id");
    const href = it.getAttribute("href");
    if (id && href) manifest.set(id, resolve(opfPath, href));
  }
  const title = byTag(opf, "title")[0]?.textContent?.trim() || "";
  const chapters: Chapter[] = [];
  for (const ref of byTag(opf, "itemref")) {
    if (ref.getAttribute("linear") === "no") continue;
    const path = manifest.get(ref.getAttribute("idref") ?? "");
    const html = path ? text(path) : null;
    if (!path || !html) continue;
    // The HTML parser forgives what XHTML files often get wrong.
    const doc = new DOMParser().parseFromString(html, "text/html");
    const body = doc.body;
    const hasText = (body.textContent ?? "").trim().length > 0;
    if (!hasText && !body.querySelector("img, image")) continue;
    const heading = body.querySelector("h1, h2, h3")?.textContent?.trim();
    chapters.push({ title: heading || doc.title.trim() || `${chapters.length + 1}`, path, body });
  }
  if (!chapters.length) throw new Error("this EPUB has no readable content");
  return { title, chapters, files };
}

type Block = { kind: "text"; text: string; level: number; pre: boolean } | { kind: "image"; src: string } | { kind: "rule" };

const BLOCK = new Set(["p", "div", "li", "blockquote", "tr", "dt", "dd", "section", "article", "figcaption", "table", "ul", "ol", "body", "figure", "header", "footer", "aside", "nav", "main"]);
const SKIP = new Set(["script", "style", "head", "title", "math"]);

function blocks(body: Element): Block[] {
  const out: Block[] = [];
  let buf = "";
  let level = 0;
  let pre = false;
  const flush = () => {
    const t = pre ? buf.replace(/^\n+|\n+$/g, "") : buf.replace(/\s+/g, " ").trim();
    if (t.trim()) out.push({ kind: "text", text: t, level, pre });
    buf = "";
  };
  const walk = (n: Node) => {
    if (n.nodeType === Node.TEXT_NODE) {
      buf += n.textContent ?? "";
      return;
    }
    if (n.nodeType !== Node.ELEMENT_NODE) return;
    const el = n as Element;
    const name = el.localName.toLowerCase();
    if (SKIP.has(name)) return;
    if (name === "img" || name === "image") {
      const src = el.getAttribute("src") ?? el.getAttribute("href") ?? el.getAttribute("xlink:href") ?? el.getAttributeNS("http://www.w3.org/1999/xlink", "href");
      flush();
      if (src) out.push({ kind: "image", src });
      return;
    }
    if (name === "svg") {
      el.querySelectorAll("image").forEach(walk);
      return;
    }
    if (name === "br") {
      if (pre) buf += "\n";
      else flush();
      return;
    }
    if (name === "hr") {
      flush();
      out.push({ kind: "rule" });
      return;
    }
    const h = /^h([1-6])$/.exec(name);
    const isBlock = BLOCK.has(name) || !!h || name === "pre";
    if (name === "td" || name === "th") buf += " ";
    if (isBlock) flush();
    const saved = { level, pre };
    if (h) level = Number(h[1]);
    if (name === "pre") pre = true;
    if (name === "li") buf += "• ";
    el.childNodes.forEach(walk);
    if (isBlock) flush();
    ({ level, pre } = saved);
  };
  walk(body);
  flush();
  return out;
}

const HEADING = [1, 1.7, 1.45, 1.25, 1.1, 1, 1];
// Canvases taller than this get split, browsers refuse very tall ones.
const MAX_PAGE = 8000;

/** Renders one chapter as canvases of the roll width, split when very long. */
export async function renderChapter(book: Book, ch: Chapter, width: number, size: number, font: FontId): Promise<HTMLCanvasElement[]> {
  const parts: HTMLCanvasElement[] = [];
  for (const b of blocks(ch.body)) {
    if (b.kind === "text") {
      if (b.level) parts.push(canvas(width, Math.round(size * 0.5)));
      const s = b.pre ? size * 0.85 : size * HEADING[b.level];
      parts.push(textBlock(b.text, width, { size: Math.round(s), bold: b.level > 0, align: "left", font: b.pre ? "mono" : font, lineHeight: 1.3 }, 4));
    } else if (b.kind === "rule") {
      const c = canvas(width, Math.round(size * 0.9));
      const x = c.getContext("2d")!;
      x.fillRect(width / 4, c.height / 2 - 1, width / 2, 2);
      parts.push(c);
    } else {
      const bytes = book.files[resolve(ch.path, b.src)];
      if (!bytes) continue;
      const url = URL.createObjectURL(new Blob([bytes as BlobPart]));
      try {
        const img = await loadImage(url);
        if (img.width < 24 || img.height < 24) continue; // icons and spacers
        const fitted = fitImage(img, Math.min(width, img.width * 2), noAdjust);
        const c = canvas(width, fitted.height + 8);
        c.getContext("2d")!.drawImage(fitted, Math.round((width - fitted.width) / 2), 4);
        parts.push(c);
      } catch {
        // unreadable picture: skip it
      } finally {
        URL.revokeObjectURL(url);
      }
    }
  }
  const pages: HTMLCanvasElement[] = [];
  let group: HTMLCanvasElement[] = [];
  let h = 0;
  const emit = () => {
    if (!group.length) return;
    const c = canvas(width, h);
    const x = c.getContext("2d")!;
    let y = 0;
    for (const p of group) {
      x.drawImage(p, 0, y);
      y += p.height;
    }
    pages.push(c);
    group = [];
    h = 0;
  };
  for (const p of parts) {
    if (h + p.height > MAX_PAGE) emit();
    group.push(p);
    h += p.height;
  }
  emit();
  return pages;
}
