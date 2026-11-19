import { useEffect, useRef, useState } from "react";
import { AlignCenter, AlignLeft, AlignRight, ArrowDown, ArrowUp, Bold, ImagePlus, Minus, QrCode, Trash2, Type } from "lucide-react";
import { t } from "../i18n";
import { canvas, FONTS, fitImage, noAdjust, qrCanvas, stack, textBlock, type Align, type FontId, type TextStyle } from "../lib/canvas";
import { imagesFromFiles } from "./Photo";
import { Empty, IconButton, Range, type ToolProps } from "./ui";

type Block =
  | { id: number; kind: "text"; text: string; style: TextStyle }
  | { id: number; kind: "image"; img: HTMLImageElement; scale: number }
  | { id: number; kind: "qr"; value: string; scale: number }
  | { id: number; kind: "sep"; dashed: boolean };

let nextId = 1;
const defaultStyle: TextStyle = { size: 28, bold: false, align: "left", font: "sans" };

async function renderBlock(b: Block, width: number): Promise<HTMLCanvasElement> {
  switch (b.kind) {
    case "text":
      return textBlock(b.text, width, b.style, 4);
    case "image": {
      const w = Math.round((width * b.scale) / 100);
      return fitImage(b.img, w, noAdjust);
    }
    case "qr":
      return qrCanvas(b.value, Math.round((width * b.scale) / 100));
    case "sep": {
      const c = canvas(width, 18);
      const x = c.getContext("2d")!;
      x.lineWidth = 3;
      if (b.dashed) x.setLineDash([10, 8]);
      x.beginPath();
      x.moveTo(0, 9);
      x.lineTo(width, 9);
      x.stroke();
      return c;
    }
  }
}

export function TextControls({ style, onChange }: { style: TextStyle; onChange: (s: TextStyle) => void }) {
  const set = (p: Partial<TextStyle>) => onChange({ ...style, ...p });
  const aligns: [Align, typeof AlignLeft][] = [
    ["left", AlignLeft],
    ["center", AlignCenter],
    ["right", AlignRight],
  ];
  return (
    <div className="row wrap">
      <select value={style.font} onChange={(e) => set({ font: e.target.value as FontId })} aria-label={t.font} title={t.font}>
        {FONTS.map((f) => (
          <option key={f.id} value={f.id}>
            {f.label}
          </option>
        ))}
      </select>
      <input
        type="number"
        min={10}
        max={200}
        value={style.size}
        onChange={(e) => set({ size: Math.max(10, Number(e.target.value) || 10) })}
        aria-label={t.size}
        title={t.size}
        className="num"
      />
      <button type="button" className={`icon-btn${style.bold ? " on" : ""}`} aria-pressed={style.bold} title={t.bold} onClick={() => set({ bold: !style.bold })}>
        <Bold size={16} />
      </button>
      {aligns.map(([a, Icon]) => (
        <button key={a} type="button" className={`icon-btn${style.align === a ? " on" : ""}`} aria-pressed={style.align === a} title={t.align[a]} onClick={() => set({ align: a })}>
          <Icon size={16} />
        </button>
      ))}
    </div>
  );
}

export function Compose({ ctx, onOutput }: ToolProps) {
  const [blocks, setBlocks] = useState<Block[]>([{ id: nextId++, kind: "text", text: "", style: { ...defaultStyle, size: 34, bold: true } }]);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let live = true;
    const parts = blocks.filter((b) => b.kind !== "text" || b.text.trim() !== "");
    Promise.all(parts.map((b) => renderBlock(b, ctx.width))).then((cs) => {
      if (live) onOutput({ pages: cs.length ? [stack(cs, ctx.width, 10)] : [], content: "image" });
    });
    return () => {
      live = false;
    };
  }, [blocks, ctx.width]);

  const patch = (id: number, p: Partial<Block>) => setBlocks((s) => s.map((b) => (b.id === id ? ({ ...b, ...p } as Block) : b)));
  const move = (i: number, d: number) =>
    setBlocks((s) => {
      const j = i + d;
      if (j < 0 || j >= s.length) return s;
      const c = [...s];
      [c[i], c[j]] = [c[j], c[i]];
      return c;
    });

  return (
    <div className="tool">
      <input
        ref={input}
        type="file"
        accept="image/*"
        multiple
        hidden
        onChange={(e) =>
          e.target.files &&
          imagesFromFiles(e.target.files).then((imgs) => setBlocks((s) => [...s, ...imgs.map((img) => ({ id: nextId++, kind: "image" as const, img, scale: 100 }))]))
        }
      />
      <div className="toolbar">
        <button type="button" className="btn" onClick={() => setBlocks((s) => [...s, { id: nextId++, kind: "text", text: "", style: { ...defaultStyle } }])}>
          <Type size={16} /> {t.addText}
        </button>
        <button type="button" className="btn" onClick={() => input.current?.click()}>
          <ImagePlus size={16} /> {t.addImage}
        </button>
        <button type="button" className="btn" onClick={() => setBlocks((s) => [...s, { id: nextId++, kind: "qr", value: "https://", scale: 60 }])}>
          <QrCode size={16} /> {t.addQr}
        </button>
        <button type="button" className="btn" onClick={() => setBlocks((s) => [...s, { id: nextId++, kind: "sep", dashed: true }])}>
          <Minus size={16} /> {t.addSeparator}
        </button>
      </div>
      {blocks.length === 0 && <Empty>{t.composeEmpty}</Empty>}
      <ul className="cards">
        {blocks.map((b, i) => (
          <li key={b.id} className="card">
            <div className="card-body">
              {b.kind === "text" && (
                <>
                  <textarea
                    value={b.text}
                    placeholder={t.textPlaceholder}
                    rows={Math.min(8, Math.max(2, b.text.split("\n").length))}
                    onChange={(e) => patch(b.id, { text: e.target.value })}
                    style={{ fontWeight: b.style.bold ? 700 : 400, textAlign: b.style.align }}
                  />
                  <TextControls style={b.style} onChange={(style) => patch(b.id, { style })} />
                </>
              )}
              {b.kind === "image" && (
                <div className="row">
                  <img src={b.img.src} alt="" className="thumb small" />
                  <Range label={t.size} min={20} max={100} value={b.scale} format={(v) => `${v} %`} onChange={(scale) => patch(b.id, { scale })} />
                </div>
              )}
              {b.kind === "qr" && (
                <div className="row">
                  <input type="text" value={b.value} onChange={(e) => patch(b.id, { value: e.target.value })} aria-label={t.value} className="grow" />
                  <Range label={t.size} min={20} max={100} value={b.scale} format={(v) => `${v} %`} onChange={(scale) => patch(b.id, { scale })} />
                </div>
              )}
              {b.kind === "sep" && (
                <div className="row">
                  <label className="check">
                    <input type="checkbox" checked={b.dashed} onChange={(e) => patch(b.id, { dashed: e.target.checked })} /> ┄
                  </label>
                </div>
              )}
            </div>
            <div className="card-side">
              <IconButton label={t.moveUp} onClick={() => move(i, -1)} disabled={i === 0}>
                <ArrowUp size={15} />
              </IconButton>
              <IconButton label={t.moveDown} onClick={() => move(i, 1)} disabled={i === blocks.length - 1}>
                <ArrowDown size={15} />
              </IconButton>
              <IconButton label={t.remove} onClick={() => setBlocks((s) => s.filter((x) => x.id !== b.id))}>
                <Trash2 size={15} />
              </IconButton>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
