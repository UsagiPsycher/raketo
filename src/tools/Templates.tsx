import { useEffect, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { t } from "../i18n";
import { canvas, fontCss, FONTS, setFont, stack, textBlock, type FontId, type TextStyle } from "../lib/canvas";
import { TextControls } from "./Compose";
import { Field, IconButton, Segmented, Toggle, type ToolProps } from "./ui";

function dateLine(width: number) {
  const s = t.date(new Date());
  return textBlock(s.charAt(0).toUpperCase() + s.slice(1), width, { size: 20, bold: false, align: "left", font: "sans" }, 4);
}

function framed(inner: HTMLCanvasElement, width: number, on: boolean): HTMLCanvasElement {
  if (!on) return inner;
  const pad = 14;
  const c = canvas(width, inner.height + pad * 2);
  const x = c.getContext("2d")!;
  x.drawImage(inner, 0, pad);
  x.lineWidth = 4;
  x.strokeRect(2, 2, width - 4, c.height - 4);
  return c;
}

export function Note({ ctx, onOutput }: ToolProps) {
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [style, setStyle] = useState<TextStyle>({ size: 30, bold: false, align: "left", font: "round" });
  const [date, setDate] = useState(true);
  const [frame, setFrame] = useState(true);

  useEffect(() => {
    if (!title.trim() && !body.trim()) return onOutput({ pages: [], content: "text" });
    const inner = ctx.width - 24;
    const parts: HTMLCanvasElement[] = [];
    if (date) parts.push(dateLine(inner));
    if (title.trim()) parts.push(textBlock(title, inner, { ...style, size: Math.round(style.size * 1.3), bold: true }, 4));
    if (body.trim()) parts.push(textBlock(body, inner, style, 4));
    const name = title.trim() || body.trim().split("\n")[0];
    onOutput({ pages: [framed(stack(parts, ctx.width, 10), ctx.width, frame)], content: "text", title: name });
  }, [title, body, style, date, frame, ctx.width]);

  return (
    <div className="tool">
      <div className="form">
        <Field label={t.noteTitle}>
          <input type="text" value={title} onChange={(e) => setTitle(e.target.value)} />
        </Field>
        <Field label={t.noteBody}>
          <textarea rows={6} value={body} onChange={(e) => setBody(e.target.value)} />
        </Field>
        <TextControls style={style} onChange={setStyle} />
        <div className="row wrap">
          <Toggle label={t.showDate} checked={date} onChange={setDate} />
          <Toggle label={t.frame} checked={frame} onChange={setFrame} />
        </div>
      </div>
    </div>
  );
}

interface Task {
  id: number;
  text: string;
  done: boolean;
}
let nextTask = 1;

type Orientation = "portrait" | "landscape";

/** Turns a band laid out along the roll so it prints lengthwise, readable from the start of the strip. */
function alongRoll(band: HTMLCanvasElement): HTMLCanvasElement {
  const out = canvas(band.height, band.width);
  const o = out.getContext("2d")!;
  o.translate(band.height, 0);
  o.rotate(Math.PI / 2);
  o.drawImage(band, 0, 0);
  return out;
}

function taskRow(it: Task, w: number, size: number, font: FontId): HTMLCanvasElement {
  const box = Math.round(size * 0.9);
  const text = textBlock(it.text, w - box - 24, { size, bold: false, align: "left", font });
  const c = canvas(w, Math.max(text.height, box + 8));
  const x = c.getContext("2d")!;
  x.lineWidth = 3;
  const by = Math.round((Math.min(text.height, size * 1.25) - box) / 2) + 2;
  x.strokeRect(6, by, box, box);
  if (it.done) {
    x.beginPath();
    x.moveTo(6 + box * 0.2, by + box * 0.55);
    x.lineTo(6 + box * 0.42, by + box * 0.78);
    x.lineTo(6 + box * 0.82, by + box * 0.22);
    x.stroke();
  }
  x.drawImage(text, box + 20, 0);
  return c;
}

/** Landscape: tasks flow in columns across the roll length, the roll width is the page height. */
function landscapeList(makeHeader: (w: number) => HTMLCanvasElement[], items: Task[], rollWidth: number, size: number, font: FontId): HTMLCanvasElement {
  const gap = 8;
  const pad = 10;
  const probe = canvas(1, 1).getContext("2d")!;
  probe.font = `400 ${size}px ${fontCss(font)}`;
  const box = Math.round(size * 0.9);
  // Column width: the longest task, between one and two roll widths.
  const longest = Math.max(0, ...items.map((i) => probe.measureText(i.text).width));
  const colW = Math.round(Math.min(rollWidth * 2, Math.max(rollWidth, longest + box + 34)));
  // The header is measured at its narrowest; laid out wider it only gets shorter.
  const minHead = Math.round(rollWidth * 1.5);
  const estimate = makeHeader(minHead);
  const headH = estimate.reduce((a, h) => a + h.height, 0) + (estimate.length ? gap : 0);
  const avail = Math.max(size * 2, rollWidth - pad * 2 - headH);
  const cols: HTMLCanvasElement[][] = [[]];
  let used = 0;
  for (const it of items) {
    const row = taskRow(it, colW, size, font);
    if (used > 0 && used + row.height > avail) {
      cols.push([]);
      used = 0;
    }
    cols[cols.length - 1].push(row);
    used += row.height + gap;
  }
  const colsW = cols.length * colW + (cols.length - 1) * 24;
  const header = makeHeader(Math.max(colsW, minHead));
  const len = Math.max(colsW, ...header.map((h) => h.width)) + pad * 2;
  const band = canvas(len, rollWidth);
  const x = band.getContext("2d")!;
  let y = pad;
  for (const h of header) {
    x.drawImage(h, pad, y);
    y += h.height;
  }
  y += header.length ? gap : 0;
  cols.forEach((col, i) => {
    let cy = y;
    const cx = pad + i * (colW + 24);
    for (const r of col) {
      x.drawImage(r, cx, cy);
      cy += r.height + gap;
    }
  });
  return alongRoll(band);
}

export function Todo({ ctx, onOutput }: ToolProps) {
  const [title, setTitle] = useState(t.todoTitle);
  const [tasks, setTasks] = useState<Task[]>([{ id: nextTask++, text: "", done: false }]);
  const [date, setDate] = useState(true);
  const [size, setSize] = useState(26);
  const [font, setFontId] = useState<FontId>("sans");
  const [orientation, setOrientation] = useState<Orientation>("portrait");

  useEffect(() => {
    const items = tasks.filter((x) => x.text.trim());
    if (!items.length && !title.trim()) return onOutput({ pages: [], content: "text" });
    const landscape = orientation === "landscape";
    const header = (w: number) => {
      const h: HTMLCanvasElement[] = [];
      if (date) h.push(dateLine(w));
      if (title.trim()) h.push(textBlock(title, w, { size: Math.round(size * 1.4), bold: true, align: "left", font }, 4));
      return h;
    };
    const page = landscape
      ? landscapeList(header, items, ctx.width, size, font)
      : stack([...header(ctx.width), ...items.map((it) => taskRow(it, ctx.width, size, font))], ctx.width, 8);
    onOutput({ pages: [page], content: "text", title: title.trim() || t.tools.todo });
  }, [title, tasks, date, size, font, orientation, ctx.width]);

  return (
    <div className="tool">
      <div className="form">
        <Field label={t.noteTitle}>
          <input type="text" value={title} onChange={(e) => setTitle(e.target.value)} />
        </Field>
        <div className="row wrap">
          <select value={font} onChange={(e) => setFontId(e.target.value as FontId)} aria-label={t.font}>
            {FONTS.map((f) => (
              <option key={f.id} value={f.id}>
                {f.label}
              </option>
            ))}
          </select>
          <input type="number" className="num" min={14} max={80} value={size} onChange={(e) => setSize(Number(e.target.value) || 14)} aria-label={t.size} title={t.size} />
          <Toggle label={t.showDate} checked={date} onChange={setDate} />
        </div>
        <Segmented<Orientation>
          label={t.orientation}
          value={orientation}
          onChange={setOrientation}
          options={[
            { value: "portrait", label: t.portrait, tip: t.portraitTip },
            { value: "landscape", label: t.landscapeTodo, tip: t.landscapeTodoTip },
          ]}
        />
      </div>
      <ul className="tasks">
        {tasks.map((it) => (
          <li key={it.id} className="task">
            <input type="checkbox" checked={it.done} onChange={(e) => setTasks((s) => s.map((x) => (x.id === it.id ? { ...x, done: e.target.checked } : x)))} aria-label={it.text || t.todoItem} />
            <input
              type="text"
              value={it.text}
              placeholder={t.todoItem}
              onChange={(e) => setTasks((s) => s.map((x) => (x.id === it.id ? { ...x, text: e.target.value } : x)))}
              onKeyDown={(e) => {
                if (e.key === "Enter") setTasks((s) => [...s, { id: nextTask++, text: "", done: false }]);
              }}
              autoFocus={it.text === "" && tasks.length > 1}
            />
            <IconButton label={t.remove} onClick={() => setTasks((s) => s.filter((x) => x.id !== it.id))}>
              <Trash2 size={15} />
            </IconButton>
          </li>
        ))}
      </ul>
      <div className="toolbar">
        <button type="button" className="btn" onClick={() => setTasks((s) => [...s, { id: nextTask++, text: "", done: false }])}>
          <Plus size={16} /> {t.addItem}
        </button>
      </div>
    </div>
  );
}

export function Banner({ ctx, onOutput }: ToolProps) {
  const [text, setText] = useState("");
  const [font, setFontId] = useState<FontId>("sans");
  const [bold, setBold] = useState(true);
  const [border, setBorder] = useState(false);

  useEffect(() => {
    if (!text.trim()) return onOutput({ pages: [], content: "text" });
    const h = ctx.width; // the roll width becomes the letter height
    const margin = border ? Math.round(h * 0.12) : Math.round(h * 0.04);
    const probe = canvas(1, 1).getContext("2d")!;
    const style: TextStyle = { size: h - margin * 2, bold, align: "left", font };
    setFont(probe, style);
    const m = probe.measureText(text);
    // Fit the cap height to the band.
    const ascent = m.actualBoundingBoxAscent || style.size * 0.75;
    const descent = m.actualBoundingBoxDescent || style.size * 0.2;
    const size = Math.floor((style.size * (h - margin * 2)) / (ascent + descent));
    const s2 = { ...style, size };
    setFont(probe, s2);
    const m2 = probe.measureText(text);
    const len = Math.ceil(m2.width) + margin * 2;
    const band = canvas(len, h);
    const x = band.getContext("2d")!;
    setFont(x, s2);
    x.font = `${bold ? 700 : 400} ${size}px ${fontCss(font)}`;
    const a2 = m2.actualBoundingBoxAscent || size * 0.75;
    const d2 = m2.actualBoundingBoxDescent || size * 0.2;
    x.fillText(text, margin, (h + a2 - d2) / 2);
    if (border) {
      x.lineWidth = Math.max(3, Math.round(h * 0.03));
      x.strokeRect(margin / 3, margin / 3, len - (margin * 2) / 3, h - (margin * 2) / 3);
    }
    // Rotate so the text runs along the paper, readable from the start of the strip.
    const out = canvas(h, len);
    const o = out.getContext("2d")!;
    o.translate(h, 0);
    o.rotate(Math.PI / 2);
    o.drawImage(band, 0, 0);
    onOutput({ pages: [out], content: "text", title: text });
  }, [text, font, bold, border, ctx.width]);

  return (
    <div className="tool">
      <div className="form">
        <Field label={t.bannerText} tip={t.bannerHint}>
          <input type="text" value={text} onChange={(e) => setText(e.target.value)} className="big" />
        </Field>
        <p className="muted">{t.bannerHint}</p>
        <div className="row wrap">
          <select value={font} onChange={(e) => setFontId(e.target.value as FontId)} aria-label={t.font}>
            {FONTS.map((f) => (
              <option key={f.id} value={f.id}>
                {f.label}
              </option>
            ))}
          </select>
          <Toggle label={t.bold} checked={bold} onChange={setBold} />
          <Toggle label={t.border} checked={border} onChange={setBorder} />
        </div>
      </div>
    </div>
  );
}
