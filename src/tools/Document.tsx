import { useEffect, useRef, useState } from "react";
import { FileUp } from "lucide-react";
import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { t } from "../i18n";
import { canvas, fileToDataUrl, fitImage, FONTS, loadImage, noAdjust, textBlock, type FontId } from "../lib/canvas";
import { openEpub, renderChapter, type Book } from "../lib/epub";
import { Empty, Field, Range, Toggle, type ToolProps } from "./ui";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

type Source =
  | { kind: "pdf"; name: string; doc: pdfjs.PDFDocumentProxy }
  | { kind: "epub"; name: string; book: Book }
  | { kind: "text"; name: string; text: string }
  | { kind: "image"; name: string; img: HTMLImageElement };

/** Parses "1-3, 5" into 1-based page numbers within `count`. */
export function parseRange(s: string, count: number): number[] {
  if (!s.trim()) return Array.from({ length: count }, (_, i) => i + 1);
  const out = new Set<number>();
  for (const part of s.split(/[,;\s]+/)) {
    const m = part.match(/^(\d+)(?:-(\d*))?$/);
    if (!m) continue;
    const a = Number(m[1]);
    const b = m[2] === undefined ? a : m[2] === "" ? count : Number(m[2]);
    for (let i = Math.max(1, a); i <= Math.min(count, b); i++) out.add(i);
  }
  return [...out].sort((x, y) => x - y);
}

async function renderPdfPage(doc: pdfjs.PDFDocumentProxy, n: number, width: number, landscape: boolean) {
  const page = await doc.getPage(n);
  const base = page.getViewport({ scale: 1 });
  const rotate = landscape && base.width > base.height ? 90 : 0;
  const vp0 = page.getViewport({ scale: 1, rotation: rotate });
  const vp = page.getViewport({ scale: width / vp0.width, rotation: rotate });
  const c = canvas(vp.width, vp.height);
  await page.render({ canvas: c, canvasContext: c.getContext("2d")!, viewport: vp, background: "#fff" }).promise;
  return c;
}

export function Document({ ctx, onOutput }: ToolProps) {
  const [src, setSrc] = useState<Source | null>(null);
  const [range, setRange] = useState("");
  const [landscape, setLandscape] = useState(false);
  const [size, setSize] = useState(22);
  const [font, setFont] = useState<FontId>("serif");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);

  const open = async (f: File) => {
    setError(null);
    setLoading(true);
    try {
      if (f.type === "application/pdf" || f.name.toLowerCase().endsWith(".pdf")) {
        const doc = await pdfjs.getDocument({ data: new Uint8Array(await f.arrayBuffer()) }).promise;
        setSrc({ kind: "pdf", name: f.name, doc });
      } else if (f.type === "application/epub+zip" || f.name.toLowerCase().endsWith(".epub")) {
        const book = openEpub(new Uint8Array(await f.arrayBuffer()));
        setSrc({ kind: "epub", name: f.name, book });
        // A whole book is a lot of paper: start with the first chapter.
        setRange("1");
        return;
      } else if (f.type.startsWith("image/")) {
        setSrc({ kind: "image", name: f.name, img: await loadImage(await fileToDataUrl(f)) });
      } else {
        setSrc({ kind: "text", name: f.name, text: await f.text() });
      }
      setRange("");
    } catch (e) {
      setError(String((e as Error)?.message ?? e));
    } finally {
      setLoading(false);
    }
  };

  const count = src?.kind === "pdf" ? src.doc.numPages : src?.kind === "epub" ? src.book.chapters.length : 1;

  useEffect(() => {
    let live = true;
    (async () => {
      if (!src) return onOutput({ pages: [], content: "text" });
      let pages: HTMLCanvasElement[] = [];
      if (src.kind === "pdf") {
        setLoading(true);
        for (const n of parseRange(range, count)) {
          if (!live) return;
          pages.push(await renderPdfPage(src.doc, n, ctx.width, landscape));
        }
        setLoading(false);
      } else if (src.kind === "epub") {
        setLoading(true);
        for (const n of parseRange(range, count)) {
          if (!live) return;
          pages.push(...(await renderChapter(src.book, src.book.chapters[n - 1], ctx.width, size, font)));
        }
        setLoading(false);
      } else if (src.kind === "image") {
        const rotate = landscape && src.img.width > src.img.height ? 90 : 0;
        pages = [fitImage(src.img, ctx.width, { ...noAdjust, rotate })];
      } else {
        pages = [textBlock(src.text, ctx.width, { size: 20, bold: false, align: "left", font: "mono", lineHeight: 1.2 }, 2)];
      }
      const title = src.kind === "epub" && src.book.title ? src.book.title : src.name.replace(/\.[^.]+$/, "");
      if (live) onOutput({ pages, content: "text", title });
    })();
    return () => {
      live = false;
    };
  }, [src, range, landscape, ctx.width, size, font]);

  return (
    <div
      className="tool"
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        const f = e.dataTransfer.files[0];
        if (f) open(f);
      }}
    >
      <input ref={input} type="file" accept=".pdf,.epub,.txt,.md,.csv,.log,image/*,text/*" hidden onChange={(e) => e.target.files?.[0] && open(e.target.files[0])} />
      <div className="toolbar">
        <button type="button" className="btn" onClick={() => input.current?.click()}>
          <FileUp size={16} /> {t.openDocument}
        </button>
        {src && <span className="muted ellipsis">{src.name}</span>}
        {loading && <span className="muted">{t.loading}</span>}
      </div>
      {error && <p className="error">{error}</p>}
      {!src ? (
        <Empty>{t.documentHint}</Empty>
      ) : (
        <div className="form">
          {src.kind === "pdf" && (
            <Field label={`${t.pages} (${count})`}>
              <input type="text" value={range} placeholder={`${t.allPages} — ${t.pageRange}`} onChange={(e) => setRange(e.target.value)} />
            </Field>
          )}
          {src.kind === "epub" && (
            <>
              <Field label={`${t.chapters} (${count})`}>
                <input type="text" value={range} placeholder={`${t.allPages} — ${t.pageRange}`} onChange={(e) => setRange(e.target.value)} />
              </Field>
              <ol className="chapters">
                {src.book.chapters.map((c, i) => (
                  <li key={c.path}>
                    <button type="button" className={`link${parseRange(range, count).includes(i + 1) ? " on" : ""}`} onClick={() => setRange(String(i + 1))}>
                      {c.title}
                    </button>
                  </li>
                ))}
              </ol>
              <div className="row wrap">
                <select value={font} onChange={(e) => setFont(e.target.value as FontId)} aria-label={t.font}>
                  {FONTS.map((f) => (
                    <option key={f.id} value={f.id}>
                      {f.label}
                    </option>
                  ))}
                </select>
              </div>
              <Range label={t.textSize} min={14} max={40} value={size} onChange={setSize} />
            </>
          )}
          {(src.kind === "pdf" || src.kind === "image") && <Toggle label={t.landscape} tip={t.landscapeTip} checked={landscape} onChange={setLandscape} />}
        </div>
      )}
    </div>
  );
}
