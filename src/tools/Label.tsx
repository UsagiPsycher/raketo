import { useEffect, useState } from "react";
import { Barcode, QrCode, Trash2, Type } from "lucide-react";
import { t } from "../i18n";
import { barcodeCanvas, canvas, mmToPx, qrCanvas, textBlock, type TextStyle } from "../lib/canvas";
import { TextControls } from "./Compose";
import { Field, IconButton, Toggle, type ToolProps } from "./ui";

type Item =
  | { id: number; kind: "text"; text: string; style: TextStyle }
  | { id: number; kind: "qr"; value: string }
  | { id: number; kind: "barcode"; value: string };

const PRESETS: [number, number][] = [
  [40, 30],
  [50, 30],
  [40, 40],
  [30, 20],
  [50, 80],
];

let nextId = 1;

export function Label({ ctx, onOutput }: ToolProps) {
  const [w, setW] = useState(40);
  const [h, setH] = useState(30);
  const [paper, setPaper] = useState(ctx.labelCapable);
  const [frame, setFrame] = useState(false);
  const [items, setItems] = useState<Item[]>([
    { id: nextId++, kind: "text", text: "", style: { size: 30, bold: true, align: "center", font: "sans" } },
  ]);

  useEffect(() => {
    let live = true;
    (async () => {
      const dpi = ctx.dpi || 203;
      // Label wider than the head: scale it down to fit.
      const lw = Math.min(mmToPx(w, dpi), ctx.width);
      const lh = Math.round((mmToPx(h, dpi) * lw) / mmToPx(w, dpi));
      const pad = Math.round(lw * 0.05);
      const inner = lw - pad * 2;
      const parts: HTMLCanvasElement[] = [];
      for (const it of items) {
        if (it.kind === "text" && it.text.trim()) parts.push(textBlock(it.text, inner, it.style));
        if (it.kind === "qr" && it.value) parts.push(await qrCanvas(it.value, Math.min(inner, Math.round(lh * 0.6))));
        if (it.kind === "barcode" && it.value) {
          const b = barcodeCanvas(it.value, inner, Math.round(lh * 0.3));
          if (b) parts.push(b);
        }
      }
      const c = canvas(ctx.width, lh);
      const x = c.getContext("2d")!;
      const left = Math.round((ctx.width - lw) / 2);
      if (frame) {
        x.lineWidth = 3;
        x.strokeRect(left + 2, 2, lw - 4, lh - 4);
      }
      const total = parts.reduce((a, p) => a + p.height, 0);
      const gap = parts.length > 1 ? Math.max(0, (lh - pad * 2 - total) / (parts.length + 1)) : 0;
      let y = parts.length > 1 ? pad + gap : Math.max(pad, (lh - total) / 2);
      for (const p of parts) {
        x.drawImage(p, left + Math.round((lw - p.width) / 2), Math.round(y));
        y += p.height + gap;
      }
      if (live) onOutput({ pages: parts.length ? [c] : [], content: "text", label: paper, labelHeightMm: h });
    })();
    return () => {
      live = false;
    };
  }, [items, w, h, frame, paper, ctx.width, ctx.dpi]);

  const patch = (id: number, p: Partial<Item>) => setItems((s) => s.map((i) => (i.id === id ? ({ ...i, ...p } as Item) : i)));

  return (
    <div className="tool">
      <div className="form">
        <Field label={t.labelSize}>
          <div className="row wrap">
            {PRESETS.map(([pw, ph]) => (
              <button key={`${pw}x${ph}`} type="button" className={`chip${pw === w && ph === h ? " on" : ""}`} onClick={() => (setW(pw), setH(ph))}>
                {pw}×{ph}
              </button>
            ))}
            <input type="number" className="num" min={10} max={120} value={w} onChange={(e) => setW(Number(e.target.value) || 10)} aria-label={t.labelWidth} title={t.labelWidth} />
            ×
            <input type="number" className="num" min={10} max={300} value={h} onChange={(e) => setH(Number(e.target.value) || 10)} aria-label={t.labelHeight} title={t.labelHeight} />
            mm
          </div>
        </Field>
        <div className="row wrap">
          <Toggle label={t.frame} checked={frame} onChange={setFrame} />
          {ctx.labelCapable && <Toggle label={t.labelPaper} tip={t.labelPaperTip} checked={paper} onChange={setPaper} />}
        </div>
      </div>
      <div className="toolbar">
        <button type="button" className="btn" onClick={() => setItems((s) => [...s, { id: nextId++, kind: "text", text: "", style: { size: 24, bold: false, align: "center", font: "sans" } }])}>
          <Type size={16} /> {t.textLine}
        </button>
        <button type="button" className="btn" onClick={() => setItems((s) => [...s, { id: nextId++, kind: "qr", value: "" }])}>
          <QrCode size={16} /> {t.qr}
        </button>
        <button type="button" className="btn" onClick={() => setItems((s) => [...s, { id: nextId++, kind: "barcode", value: "" }])}>
          <Barcode size={16} /> {t.barcode}
        </button>
      </div>
      <ul className="cards">
        {items.map((it) => (
          <li key={it.id} className="card">
            <div className="card-body">
              {it.kind === "text" ? (
                <>
                  <input type="text" value={it.text} placeholder={t.textPlaceholder} onChange={(e) => patch(it.id, { text: e.target.value })} />
                  <TextControls style={it.style} onChange={(style) => patch(it.id, { style })} />
                </>
              ) : (
                <Field label={it.kind === "qr" ? t.qr : t.barcode}>
                  <input type="text" value={it.value} placeholder={t.value} onChange={(e) => patch(it.id, { value: e.target.value })} />
                </Field>
              )}
            </div>
            <div className="card-side">
              <IconButton label={t.remove} onClick={() => setItems((s) => s.filter((x) => x.id !== it.id))}>
                <Trash2 size={15} />
              </IconButton>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
