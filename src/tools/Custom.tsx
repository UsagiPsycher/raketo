import { useEffect, useRef, useState, type DragEvent, type PointerEvent as ReactPointerEvent } from "react";
import {
  AlignCenter,
  AlignLeft,
  AlignRight,
  BringToFront,
  Circle,
  Copy,
  FilePlus2,
  FolderOpen,
  FlipHorizontal2,
  ImagePlus,
  Italic,
  Minus,
  Bold,
  Redo2,
  RotateCcw,
  RotateCw,
  Save,
  SendToBack,
  Square,
  Trash2,
  Type,
  Underline,
  Undo2,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { api } from "../api";
import { t } from "../i18n";
import { fileToDataUrl, loadImage, type Align } from "../lib/canvas";
import {
  bounds,
  emptyLayout,
  LINE,
  lineBetween,
  lineEnds,
  makeItem,
  MIN_LENGTH,
  newId,
  PAD,
  paperLength,
  parse,
  PT,
  renderItem,
  renderLayout,
  serialize,
  straighten,
  textFont,
  textHeight,
  type ImageItem,
  type Item,
  type ItemType,
  type Layout,
  type ShapeItem,
  type TextItem,
} from "../lib/rkt";
import { FontPicker } from "./FontPicker";
import { Range, Segmented, Toggle, type ToolProps } from "./ui";

const STORE = "raketo.custom";
const ZOOMS = [0.5, 0.75, 1, 1.25, 1.5, 2];
const DRAG_TYPE = "application/x-raketo-item";
const MIN = 1; // mm, smallest box side

type History = { past: Layout[]; now: Layout; future: Layout[] };
type Drag =
  | { kind: "move"; id: number; start: Item; x0: number; y0: number; moved: boolean }
  | { kind: "resize"; id: number; start: Item; x0: number; y0: number; hx: number; hy: number }
  | { kind: "rotate"; id: number; start: Item; cx: number; cy: number }
  | { kind: "end"; id: number; start: Item; end: 0 | 1; fixed: [number, number] }
  | { kind: "length" };
type Pt = [number, number];
/** A line being drawn: first click made, waiting for the second one. */
type Draft = { from: Pt; to: Pt; x0: number; y0: number; pressed: boolean; dragged: boolean };

const round = (v: number, d = 10) => Math.round(v * d) / d;

/** Pictures are kept at most this many pixels on their long side, so files stay small. */
const MAX_IMAGE = 1600;

async function importImage(f: File): Promise<{ src: string; img: HTMLImageElement } | null> {
  if (!f.type.startsWith("image/")) return null;
  let src = await fileToDataUrl(f);
  let img = await loadImage(src);
  const long = Math.max(img.width, img.height);
  if (long > MAX_IMAGE) {
    const k = MAX_IMAGE / long;
    const c = document.createElement("canvas");
    c.width = Math.round(img.width * k);
    c.height = Math.round(img.height * k);
    c.getContext("2d")!.drawImage(img, 0, 0, c.width, c.height);
    src = f.type === "image/png" ? c.toDataURL("image/png") : c.toDataURL("image/jpeg", 0.9);
    img = await loadImage(src);
  }
  return { src, img };
}

function loadStored(width: number): Layout {
  try {
    const s = localStorage.getItem(STORE);
    if (s) return parse(s);
  } catch {
    // nothing stored, or unreadable
  }
  return emptyLayout(width);
}

/** One item drawn exactly as it will print, scaled by CSS. */
function ItemView({ item, s, images, version }: { item: Item; s: number; images: Map<string, HTMLImageElement>; version: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  // Position and rotation are applied by CSS, so moving does not redraw.
  const { x: _x, y: _y, rotate: _r, id: _id, ...look } = item;
  const key = JSON.stringify(look);
  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const part = renderItem(item, s, images);
    c.width = part.width;
    c.height = part.height;
    c.getContext("2d")!.drawImage(part, 0, 0);
  }, [key, s, version]);
  return <canvas ref={ref} />;
}

export function Custom({ ctx, onOutput }: ToolProps) {
  const paperMm = round((ctx.width / ctx.dpi) * 25.4, 100);
  const [hist, setHist] = useState<History>(() => ({ past: [], now: loadStored(paperMm), future: [] }));
  const [selected, setSelected] = useState<number | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  // null: the paper fits the space next to the panel.
  const [manualZoom, setManualZoom] = useState<number | null>(null);
  const [room, setRoom] = useState(0);
  const [guides, setGuides] = useState<number[]>([]);
  const [name, setName] = useState(() => {
    try {
      return localStorage.getItem(`${STORE}.name`) || "";
    } catch {
      return "";
    }
  });
  const [note, setNote] = useState<{ kind: "ok" | "error"; text: string } | null>(null);
  const [dropping, setDropping] = useState(false);
  // Line drawing mode, entered from the bar, like the line tool of a paint program.
  const [drawMode, setDrawMode] = useState<{ dashed: boolean } | null>(null);
  const [draft, setDraftState] = useState<Draft | null>(null);
  const draftRef = useRef<Draft | null>(null);
  const setDraft = (d: Draft | null) => {
    draftRef.current = d;
    setDraftState(d ? { ...d } : null);
  };
  useEffect(() => {
    if (!note) return;
    const id = setTimeout(() => setNote(null), 5000);
    return () => clearTimeout(id);
  }, [note]);
  const images = useRef(new Map<string, HTMLImageElement>());
  const [version, setVersion] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const paper = useRef<HTMLDivElement>(null);
  const imageInput = useRef<HTMLInputElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const drag = useRef<Drag | null>(null);
  const lastKey = useRef<{ key: string; at: number } | null>(null);

  const body = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = body.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setRoom(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const stacked = room > 0 && room < 600;
  // Ruler, gaps and the properties panel take the rest.
  const fitZoom = room ? Math.min(1.5, Math.max(0.4, (room - (stacked ? 40 : 40 + 16 + 224)) / ctx.width)) : 1;
  const zoom = manualZoom ?? fitZoom;
  const zoomBy = (dir: 1 | -1) => {
    const next = dir > 0 ? ZOOMS.find((z) => z > zoom + 0.01) : [...ZOOMS].reverse().find((z) => z < zoom - 0.01);
    if (next) setManualZoom(next);
  };

  const layout = hist.now;
  const s = ctx.width / layout.width; // printer pixels per mm
  const k = s * zoom; // screen pixels per mm
  const length = paperLength(layout);
  const sel = layout.items.find((i) => i.id === selected) ?? null;

  // Keeps a fresh copy of the state for the window listeners.
  const live = useRef({ hist, selected, editing, k, layout, drawMode });
  live.current = { hist, selected, editing, k, layout, drawMode };

  /** Records a change. Changes with the same key in quick succession make one undo step. */
  const commit = (next: Layout, key?: string) => {
    const now = Date.now();
    const merge = key && lastKey.current?.key === key && now - lastKey.current.at < 1000;
    lastKey.current = key ? { key, at: now } : null;
    setHist((h) => ({ past: merge ? h.past : [...h.past.slice(-80), h.now], now: next, future: [] }));
  };
  /** Changes during a drag: one undo step is recorded when it starts. */
  const preview = (next: Layout) => setHist((h) => ({ ...h, now: next }));
  const checkpoint = () => {
    lastKey.current = null;
    setHist((h) => ({ past: [...h.past.slice(-80), h.now], now: h.now, future: [] }));
  };
  const undo = () =>
    setHist((h) => (h.past.length ? { past: h.past.slice(0, -1), now: h.past[h.past.length - 1], future: [h.now, ...h.future] } : h));
  const redo = () => setHist((h) => (h.future.length ? { past: [...h.past, h.now], now: h.future[0], future: h.future.slice(1) } : h));

  const withItem = (l: Layout, id: number, f: (i: Item) => Item): Layout => ({ ...l, items: l.items.map((i) => (i.id === id ? fit(f(i)) : i)) });
  const patch = (id: number, p: Partial<Item>, key = `patch-${id}-${Object.keys(p).join()}`) => commit(withItem(layout, id, (i) => ({ ...i, ...p }) as Item), key);

  // An empty layout follows the printer width.
  useEffect(() => {
    if (!layout.items.length && Math.abs(layout.width - paperMm) > 0.01) setHist((h) => ({ ...h, now: { ...h.now, width: paperMm } }));
  }, [paperMm, layout.items.length]);

  // Loads pictures used by the layout.
  useEffect(() => {
    for (const [key, src] of Object.entries(layout.images)) {
      if (images.current.has(key)) continue;
      loadImage(src).then(
        (img) => {
          images.current.set(key, img);
          setVersion((v) => v + 1);
        },
        () => {},
      );
    }
  }, [layout.images]);

  // Saved in the browser between sessions.
  useEffect(() => {
    const id = setTimeout(() => {
      try {
        localStorage.setItem(STORE, serialize(layout));
        localStorage.setItem(`${STORE}.name`, name);
      } catch {
        // storage full or blocked: the layout simply is not kept
      }
    }, 600);
    return () => clearTimeout(id);
  }, [layout, name]);

  // What gets printed, redrawn shortly after changes stop.
  useEffect(() => {
    const visible = layout.items.filter((i) => i.type !== "text" || i.text.trim() || i.invert);
    if (!visible.length) {
      onOutput({ pages: [], content: "text" });
      return;
    }
    const id = setTimeout(() => {
      const page = renderLayout({ ...layout, items: visible }, ctx.width, images.current);
      onOutput({ pages: [page], content: visible.some((i) => i.type === "image") ? "photo" : "text", title: name || t.tools.custom });
    }, 200);
    return () => clearTimeout(id);
  }, [layout, ctx.width, version, name]);

  /** Text boxes take the height their text needs. */
  function fit(i: Item): Item {
    if (i.type !== "text") return i;
    const h = textHeight(i);
    return Math.abs(h - i.h) < 0.01 ? i : { ...i, h };
  }

  /** Where the next item goes when added with a click: under the others. */
  const nextSpot = (h: number) => {
    const bottom = layout.items.reduce((m, i) => Math.max(m, bounds(i).bottom), 0);
    return { cx: layout.width / 2, cy: (layout.items.length ? bottom + 2 : 3) + h / 2 };
  };

  const add = (type: ItemType, at?: { cx: number; cy: number }) => {
    if (type === "image") return imageInput.current?.click();
    const probe = makeItem(type, 0, 0, layout.width);
    const spot = at ?? nextSpot(type === "text" ? 6 : probe.h);
    let item = makeItem(type, spot.cx, spot.cy, layout.width);
    if (item.type === "text") item = fit({ ...item, text: t.customText });
    commit({ ...layout, items: [...layout.items, item] });
    setSelected(item.id);
    if (item.type === "text") setEditing(item.id);
  };

  const addImages = async (files: Iterable<File>, at?: { cx: number; cy: number }) => {
    let l = live.current.layout;
    let last: number | null = null;
    for (const f of files) {
      const got = await importImage(f).catch(() => null);
      if (!got) continue;
      const key = `img${Date.now().toString(36)}${newId()}`;
      images.current.set(key, got.img);
      const w = Math.min(l.width * 0.8, (got.img.width / ctx.dpi) * 25.4 * 2);
      const h = (w * got.img.height) / got.img.width;
      const bottom = l.items.reduce((m, i) => Math.max(m, bounds(i).bottom), 0);
      const spot: { cx: number; cy: number } = at && last === null ? at : { cx: l.width / 2, cy: (l.items.length ? bottom + 2 : 2) + h / 2 };
      const item: ImageItem = { ...(makeItem("image", spot.cx, spot.cy, l.width) as ImageItem), image: key, x: spot.cx - w / 2, y: spot.cy - h / 2, w, h };
      l = { ...l, images: { ...l.images, [key]: got.src }, items: [...l.items, item] };
      last = item.id;
    }
    if (last === null) return;
    commit(l);
    setSelected(last);
    setVersion((v) => v + 1);
  };

  const remove = (id: number) => {
    commit({ ...layout, items: layout.items.filter((i) => i.id !== id) });
    setSelected(null);
    setEditing(null);
  };
  const duplicate = (id: number) => {
    const it = layout.items.find((i) => i.id === id);
    if (!it) return;
    const copy = { ...it, id: newId(), x: it.x + 2, y: it.y + 2 };
    commit({ ...layout, items: [...layout.items, copy] });
    setSelected(copy.id);
  };
  const reorder = (id: number, front: boolean) => {
    const it = layout.items.find((i) => i.id === id);
    if (!it) return;
    const rest = layout.items.filter((i) => i.id !== id);
    commit({ ...layout, items: front ? [...rest, it] : [it, ...rest] });
  };

  const openFile = async (f: File) => {
    try {
      const l = parse(await f.text());
      commit(l);
      setSelected(null);
      setEditing(null);
      setName(f.name.replace(/\.rkt$/i, ""));
      setNote({ kind: "ok", text: t.customOpened });
    } catch (e) {
      setNote({ kind: "error", text: t.customOpenFailed(String((e as Error).message ?? e)) });
    }
  };
  const save = async () => {
    try {
      const where = await api.saveLayout(`${name || t.tools.custom}.rkt`, serialize(layout));
      if (where) {
        setName(where.replace(/^.*[\\/]/, "").replace(/\.rkt$/i, ""));
        setNote({ kind: "ok", text: t.customSaved });
      }
    } catch (e) {
      setNote({ kind: "error", text: String(e) });
    }
  };
  const reset = () => {
    commit(emptyLayout(paperMm));
    setSelected(null);
    setEditing(null);
    setName("");
    setNote({ kind: "ok", text: t.customCleared });
  };

  // Pointer handling: move, resize and rotate the selected item.
  const toMm = (clientX: number, clientY: number) => {
    const r = paper.current!.getBoundingClientRect();
    return { mx: (clientX - r.left) / live.current.k, my: (clientY - r.top) / live.current.k };
  };

  const finishLine = (from: Pt, to: Pt) => {
    setDraft(null);
    if (Math.hypot(to[0] - from[0], to[1] - from[1]) < 0.5) return;
    const { layout, drawMode } = live.current;
    const base = makeItem("line", 0, 0, layout.width, { dashed: !!drawMode?.dashed });
    const item = lineBetween(base, from[0], from[1], to[0], to[1]);
    commit({ ...layout, items: [...layout.items, item] });
    setSelected(item.id);
  };
  const startDrawing = (dashed: boolean) => {
    setDraft(null);
    setEditing(null);
    setDrawMode((m) => (m && m.dashed === dashed ? null : { dashed }));
  };
  /** In drawing mode, clicks on the paper place the ends of a line instead of selecting. */
  const onPaperDownCapture = (e: ReactPointerEvent) => {
    if (!drawMode || e.button !== 0 || (e.target as HTMLElement).classList.contains("paper-grip")) return;
    e.stopPropagation();
    e.preventDefault();
    const { mx, my } = toMm(e.clientX, e.clientY);
    const d = draftRef.current;
    if (!d) setDraft({ from: [mx, my], to: [mx, my], x0: e.clientX, y0: e.clientY, pressed: true, dragged: false });
    else finishLine(d.from, e.shiftKey ? straighten(d.from, [mx, my]) : [mx, my]);
  };
  const startEnd = (e: ReactPointerEvent, it: Item, end: 0 | 1) => {
    e.stopPropagation();
    e.preventDefault();
    checkpoint();
    drag.current = { kind: "end", id: it.id, start: it, end, fixed: lineEnds(it)[end === 0 ? 1 : 0] };
  };
  const startLength = (e: ReactPointerEvent) => {
    e.stopPropagation();
    e.preventDefault();
    checkpoint();
    drag.current = { kind: "length" };
  };

  const startMove = (e: ReactPointerEvent, it: Item) => {
    if (e.button !== 0 || editing === it.id) return;
    e.stopPropagation();
    setSelected(it.id);
    if (editing !== null) setEditing(null);
    drag.current = { kind: "move", id: it.id, start: it, x0: e.clientX, y0: e.clientY, moved: false };
  };
  const startResize = (e: ReactPointerEvent, it: Item, hx: number, hy: number) => {
    e.stopPropagation();
    e.preventDefault();
    checkpoint();
    drag.current = { kind: "resize", id: it.id, start: it, x0: e.clientX, y0: e.clientY, hx, hy };
  };
  const startRotate = (e: ReactPointerEvent, it: Item) => {
    e.stopPropagation();
    e.preventDefault();
    checkpoint();
    const r = paper.current!.getBoundingClientRect();
    drag.current = { kind: "rotate", id: it.id, start: it, cx: r.left + (it.x + it.w / 2) * k, cy: r.top + (it.y + it.h / 2) * k };
  };

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const dr = draftRef.current;
      if (dr && paper.current) {
        const { mx, my } = toMm(e.clientX, e.clientY);
        if (dr.pressed && Math.hypot(e.clientX - dr.x0, e.clientY - dr.y0) > 4) dr.dragged = true;
        setDraft({ ...dr, to: e.shiftKey ? straighten(dr.from, [mx, my]) : [mx, my] });
        return;
      }
      const d = drag.current;
      if (!d) return;
      const { k, layout } = live.current;
      if (d.kind === "length") {
        const { my } = toMm(e.clientX, e.clientY);
        preview({ ...layout, length: Math.max(MIN_LENGTH, Math.round(my)) });
        return;
      }
      const st = d.start;
      if (d.kind === "end") {
        const { mx, my } = toMm(e.clientX, e.clientY);
        const p: Pt = e.shiftKey ? straighten(d.fixed, [mx, my]) : [mx, my];
        const [a, b] = d.end === 0 ? [p, d.fixed] : [d.fixed, p];
        const next = lineBetween(st, a[0], a[1], b[0], b[1]);
        preview({ ...layout, items: layout.items.map((i) => (i.id === d.id ? next : i)) });
        return;
      }
      if (d.kind === "move") {
        const dx = (e.clientX - d.x0) / k;
        const dy = (e.clientY - d.y0) / k;
        if (!d.moved) {
          if (Math.hypot(dx, dy) * k < 3) return;
          d.moved = true;
          checkpoint();
        }
        let x = st.x + dx;
        const y = st.y + dy;
        // Snaps to the paper edges and centre.
        const lines: number[] = [];
        if (!e.altKey) {
          const b = bounds({ ...st, x, y });
          const tol = 5 / k;
          const W = layout.width;
          const cands: [number, number][] = [
            [b.left, 0],
            [b.right, W],
            [(b.left + b.right) / 2, W / 2],
          ];
          for (const [v, target] of cands) {
            if (Math.abs(v - target) < tol) {
              x += target - v;
              lines.push(target);
              break;
            }
          }
        }
        setGuides(lines);
        preview({ ...layout, items: layout.items.map((i) => (i.id === d.id ? { ...i, x, y } : i)) });
      } else if (d.kind === "resize") {
        const a = (-st.rotate * Math.PI) / 180;
        const wx = (e.clientX - d.x0) / k;
        const wy = (e.clientY - d.y0) / k;
        const lx = wx * Math.cos(a) - wy * Math.sin(a);
        const ly = wx * Math.sin(a) + wy * Math.cos(a);
        let w = d.hx ? Math.max(MIN, st.w + d.hx * lx) : st.w;
        let h = d.hy ? Math.max(MIN, st.h + d.hy * ly) : st.h;
        let next: Item;
        if (st.type === "text") {
          next = d.hy ? { ...st, w, size: Math.max(0.8, (st.size * w) / st.w) } : { ...st, w };
          next = fit(next);
          h = next.h;
        } else {
          // Pictures keep their proportions from the corners, shapes with Shift.
          const corner = d.hx !== 0 && d.hy !== 0;
          if (corner && (st.type === "image") !== e.shiftKey) {
            const f = Math.max(w / st.w, h / st.h);
            w = Math.max(MIN, st.w * f);
            h = Math.max(MIN, st.h * f);
          }
          next = { ...st, w, h } as Item;
        }
        // The opposite side stays in place.
        const sx = (d.hx * (w - st.w)) / 2;
        const sy = (d.hy * (h - st.h)) / 2;
        const b = -a;
        const cx = st.x + st.w / 2 + sx * Math.cos(b) - sy * Math.sin(b);
        const cy = st.y + st.h / 2 + sx * Math.sin(b) + sy * Math.cos(b);
        next = { ...next, x: cx - w / 2, y: cy - h / 2 } as Item;
        preview({ ...layout, items: layout.items.map((i) => (i.id === d.id ? next : i)) });
      } else {
        let deg = (Math.atan2(e.clientY - d.cy, e.clientX - d.cx) * 180) / Math.PI + 90;
        if (!e.altKey) {
          const snap = Math.round(deg / 15) * 15;
          if (Math.abs(snap - deg) < 4) deg = snap;
        }
        deg = ((Math.round(deg) % 360) + 540) % 360 - 180;
        preview({ ...layout, items: layout.items.map((i) => (i.id === d.id ? { ...i, rotate: deg } : i)) });
      }
    };
    const onUp = () => {
      const dr = draftRef.current;
      if (dr?.pressed) {
        // Press, drag and release draws the line in one go; a plain click waits for the second one.
        if (dr.dragged) finishLine(dr.from, dr.to);
        else draftRef.current = { ...dr, pressed: false };
      }
      drag.current = null;
      setGuides([]);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  }, []);

  // Keyboard shortcuts, only while this tool is shown and no field has the focus.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!root.current?.offsetParent) return;
      const el = e.target as HTMLElement;
      const typing = el.closest("input, textarea, select, [contenteditable]");
      const mod = e.ctrlKey || e.metaKey;
      const key = e.key.toLowerCase();
      if (mod && key === "z" && !typing) {
        e.preventDefault();
        return e.shiftKey ? redo() : undo();
      }
      if (mod && key === "y" && !typing) return e.preventDefault(), redo();
      if (typing) return;
      const { selected, layout, drawMode } = live.current;
      if (key === "escape") {
        if (draftRef.current) return setDraft(null);
        if (drawMode) return setDrawMode(null);
        return setSelected(null);
      }
      if (selected === null) return;
      const it = layout.items.find((i) => i.id === selected);
      if (!it) return;
      if (key === "delete" || key === "backspace") return e.preventDefault(), remove(selected);
      if (mod && key === "d") return e.preventDefault(), duplicate(selected);
      if (key === "enter" && it.type === "text") return e.preventDefault(), setEditing(selected);
      const step = e.shiftKey ? 5 : 0.5;
      const moves: Record<string, [number, number]> = { arrowleft: [-step, 0], arrowright: [step, 0], arrowup: [0, -step], arrowdown: [0, step] };
      if (moves[key]) {
        e.preventDefault();
        const [dx, dy] = moves[key];
        commit(withItem(layout, selected, (i) => ({ ...i, x: i.x + dx, y: i.y + dy })), `nudge-${selected}`);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Pasted pictures land on the layout.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if (!root.current?.offsetParent || (e.target as HTMLElement).closest?.("input, textarea")) return;
      const files = [...(e.clipboardData?.files ?? [])];
      if (files.length) {
        e.preventDefault();
        addImages(files);
      }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  });

  const onDragOver = (e: DragEvent) => {
    if (![...e.dataTransfer.types].some((ty) => ty === DRAG_TYPE || ty === "Files")) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    setDropping(true);
  };
  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setDropping(false);
    const { mx, my } = toMm(e.clientX, e.clientY);
    const at = { cx: mx, cy: my };
    const idx = e.dataTransfer.getData(DRAG_TYPE);
    if (idx !== "") return addFromPalette(palette[Number(idx)], at);
    const files = [...e.dataTransfer.files];
    const rkt = files.find((f) => /\.rkt$/i.test(f.name));
    if (rkt) return openFile(rkt);
    addImages(files, at);
  };

  const palette: { type: ItemType; label: string; icon: typeof Type; extra?: Partial<ShapeItem>; draw?: boolean }[] = [
    { type: "text", label: t.customAdd.text, icon: Type },
    { type: "image", label: t.customAdd.image, icon: ImagePlus },
    { type: "line", label: t.customAdd.line, icon: Minus, draw: true },
    { type: "line", label: t.customAdd.dashed, icon: Minus, extra: { dashed: true }, draw: true },
    { type: "ellipse", label: t.customAdd.ellipse, icon: Circle },
    { type: "rect", label: t.customAdd.rect, icon: Square },
  ];
  const addFromPalette = (p: (typeof palette)[number], at?: { cx: number; cy: number }) => {
    if (p.draw) return startDrawing(!!p.extra?.dashed);
    setDrawMode(null);
    if (!p.extra) return add(p.type, at);
    const spot = at ?? nextSpot(0.6);
    const item = makeItem(p.type, spot.cx, spot.cy, layout.width, p.extra);
    commit({ ...layout, items: [...layout.items, item] });
    setSelected(item.id);
  };

  return (
    <div className="tool custom-tool" ref={root}>
      <input ref={imageInput} type="file" accept="image/*" multiple hidden onChange={(e) => (e.target.files && addImages([...e.target.files]), (e.target.value = ""))} />
      <input ref={fileInput} type="file" accept=".rkt,application/json" hidden onChange={(e) => (e.target.files?.[0] && openFile(e.target.files[0]), (e.target.value = ""))} />
      <div className="toolbar">
        <button type="button" className="btn" onClick={reset} title={t.customNewTip}>
          <FilePlus2 size={16} /> {t.customNew}
        </button>
        <button type="button" className="btn" onClick={() => fileInput.current?.click()}>
          <FolderOpen size={16} /> {t.customOpen}
        </button>
        <button type="button" className="btn" onClick={save} disabled={!layout.items.length}>
          <Save size={16} /> {t.customSave}
        </button>
        <span className="spacer" />
        <button type="button" className="icon-btn" onClick={undo} disabled={!hist.past.length} title={t.undo} aria-label={t.undo}>
          <Undo2 size={16} />
        </button>
        <button type="button" className="icon-btn" onClick={redo} disabled={!hist.future.length} title={t.redo} aria-label={t.redo}>
          <Redo2 size={16} />
        </button>
        <span className="zoom">
          <button type="button" className="icon-btn" onClick={() => zoomBy(-1)} disabled={zoom <= ZOOMS[0] + 0.01} title={t.zoomOut} aria-label={t.zoomOut}>
            <ZoomOut size={16} />
          </button>
          <button type="button" className={`btn ghost small zoom-value${manualZoom === null ? " on" : ""}`} onClick={() => setManualZoom(null)} title={t.zoomFit} aria-live="polite">
            {Math.round(zoom * 100)} %
          </button>
          <button type="button" className="icon-btn" onClick={() => zoomBy(1)} disabled={zoom >= ZOOMS[ZOOMS.length - 1] - 0.01} title={t.zoomIn} aria-label={t.zoomIn}>
            <ZoomIn size={16} />
          </button>
        </span>
      </div>

      <div className="palette" role="toolbar" aria-label={t.customInsert}>
        {palette.map((p, i) => (
          <button
            key={p.label}
            type="button"
            className={`palette-btn${p.extra?.dashed ? " dashed" : ""}${p.draw && drawMode && drawMode.dashed === !!p.extra?.dashed ? " on" : ""}`}
            title={p.draw ? t.customLineTip : t.customPaletteTip}
            aria-pressed={p.draw ? !!drawMode && drawMode.dashed === !!p.extra?.dashed : undefined}
            draggable={!p.draw}
            onDragStart={(e) => {
              e.dataTransfer.setData(DRAG_TYPE, String(i));
              e.dataTransfer.effectAllowed = "copy";
            }}
            onClick={() => addFromPalette(p)}
          >
            <p.icon size={18} strokeWidth={1.75} />
            <span>{p.label}</span>
          </button>
        ))}
      </div>

      <div className={`custom-body${stacked ? " stacked" : ""}`} ref={body}>
        <div className="paper-col">
          {drawMode && (
            <p className="draw-hint small" role="status">
              {draft ? t.customDrawEnd : t.customDrawStart}
            </p>
          )}
          <div className="paper-frame">
            <div className="ruler" style={{ height: length * k, ["--mm" as string]: `${k}px` }} aria-hidden>
              {Array.from({ length: Math.floor(length / 10) + 1 }, (_, i) => (
                <span key={i} style={{ top: i * 10 * k }}>
                  {i ? i * 10 : ""}
                </span>
              ))}
            </div>
            <div
              ref={paper}
              className={`paper${dropping ? " dropping" : ""}${layout.length ? " fixed" : ""}${drawMode ? " drawing" : ""}`}
              style={{ width: layout.width * k, height: length * k }}
              onPointerDownCapture={onPaperDownCapture}
              onPointerDown={(e) => {
                const el = e.target as HTMLElement;
                if (el === e.currentTarget || el.classList.contains("paper-items")) {
                  setSelected(null);
                  setEditing(null);
                }
              }}
              onDragOver={onDragOver}
              onDragLeave={() => setDropping(false)}
              onDrop={onDrop}
            >
              <div className="paper-items">
              {layout.items.map((it) => (
                <div
                  key={it.id}
                  className={`item ${it.type}${it.id === selected ? " selected" : ""}${it.type === "text" && !it.text.trim() ? " empty" : ""}`}
                  style={{ left: it.x * k, top: it.y * k, width: it.w * k, height: it.h * k, transform: `rotate(${it.rotate}deg)` }}
                  onPointerDown={(e) => startMove(e, it)}
                  onDoubleClick={() => it.type === "text" && setEditing(it.id)}
                >
                  {editing === it.id && it.type === "text" ? (
                    <TextEditor item={it} k={k} onChange={(text) => patch(it.id, { text }, `text-${it.id}`)} onDone={() => setEditing(null)} />
                  ) : (
                    <ItemView item={it} s={s} images={images.current} version={version} />
                  )}
                </div>
              ))}
              </div>
              {sel && editing !== sel.id && !drawMode && <Handles item={sel} k={k} onResize={startResize} onRotate={startRotate} onEnd={startEnd} />}
              {draft && (
                <svg className="draft" width={layout.width * k} height={length * k} aria-hidden>
                  <line
                    x1={draft.from[0] * k}
                    y1={draft.from[1] * k}
                    x2={draft.to[0] * k}
                    y2={draft.to[1] * k}
                    strokeWidth={Math.max(1, 0.6 * k)}
                    strokeDasharray={drawMode?.dashed ? `${Math.max(3, 1.8 * k)} ${Math.max(2, 1.2 * k)}` : undefined}
                  />
                  <circle cx={draft.from[0] * k} cy={draft.from[1] * k} r={4} />
                </svg>
              )}
              <div
                className="paper-grip"
                title={t.customGripTip}
                onPointerDown={startLength}
                onDoubleClick={() => commit({ ...layout, length: null })}
                role="separator"
                aria-orientation="horizontal"
                aria-label={t.customLength}
              />
              {guides.map((g) => (
                <span key={g} className="guide" style={{ left: g * k }} />
              ))}
              {!layout.items.length && <p className="paper-hint">{t.customEmpty}</p>}
            </div>
          </div>
          <div className="paper-end" style={{ width: layout.width * k }} aria-hidden />
          <p className="muted small paper-size">
            {t.customSize(round(layout.width, 10), length)}
            {!layout.length && <> {t.customAuto}</>}
          </p>
        </div>

        <aside className="props" aria-label={t.customProps}>
          {sel ? (
            <ItemProps
              key={sel.id}
              item={sel}
              images={images.current}
              onPatch={(p, key) => patch(sel.id, p, key)}
              onRemove={() => remove(sel.id)}
              onDuplicate={() => duplicate(sel.id)}
              onFront={() => reorder(sel.id, true)}
              onBack={() => reorder(sel.id, false)}
            />
          ) : (
            <PaperProps layout={layout} onChange={(l, key) => commit(l, key)} />
          )}
          {note && <p className={note.kind === "error" ? "error" : "ok"}>{note.text}</p>}
        </aside>
      </div>
    </div>
  );
}

function TextEditor({ item, k, onChange, onDone }: { item: TextItem; k: number; onChange: (v: string) => void; onDone: () => void }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return (
    <textarea
      ref={ref}
      className={`text-edit${item.invert ? " inverted" : ""}`}
      value={item.text}
      aria-label={t.customTextField}
      spellCheck={false}
      onChange={(e) => onChange(e.target.value)}
      onBlur={onDone}
      onKeyDown={(e) => {
        if (e.key === "Escape") onDone();
        e.stopPropagation();
      }}
      onPointerDown={(e) => e.stopPropagation()}
      style={{
        font: textFont(item, item.size * k),
        lineHeight: `${item.size * LINE * k}px`,
        padding: PAD * k,
        textAlign: item.align,
        textDecoration: item.underline ? "underline" : undefined,
      }}
    />
  );
}

function Handles({
  item,
  k,
  onResize,
  onRotate,
  onEnd,
}: {
  item: Item;
  k: number;
  onResize: (e: ReactPointerEvent, it: Item, hx: number, hy: number) => void;
  onRotate: (e: ReactPointerEvent, it: Item) => void;
  onEnd: (e: ReactPointerEvent, it: Item, end: 0 | 1) => void;
}) {
  if (item.type === "line") {
    // Lines are edited by their two ends, in any direction.
    return (
      <>
        {lineEnds(item).map(([x, y], i) => (
          <span key={i} className="handle end" title={t.customEndTip} style={{ left: x * k, top: y * k }} onPointerDown={(e) => onEnd(e, item, i as 0 | 1)} />
        ))}
      </>
    );
  }
  const sides: [number, number][] =
    item.type === "text"
        ? [
            [-1, 0],
            [1, 0],
            [-1, -1],
            [1, -1],
            [1, 1],
            [-1, 1],
          ]
        : [
            [-1, -1],
            [0, -1],
            [1, -1],
            [1, 0],
            [1, 1],
            [0, 1],
            [-1, 1],
            [-1, 0],
          ];
  return (
    <div className="handles" style={{ left: item.x * k, top: item.y * k, width: item.w * k, height: item.h * k, transform: `rotate(${item.rotate}deg)` }}>
      {sides.map(([hx, hy]) => (
        <span
          key={`${hx}${hy}`}
          className="handle"
          data-h={`${hx}${hy}`}
          style={{ left: `${((hx + 1) / 2) * 100}%`, top: `${((hy + 1) / 2) * 100}%` }}
          onPointerDown={(e) => onResize(e, item, hx, hy)}
        />
      ))}
      <span className="rotate-handle" title={t.customRotateTip} onPointerDown={(e) => onRotate(e, item)} />
    </div>
  );
}

function Num({ label, value, onChange, step = 0.5, min, max, unit = "mm" }: { label: string; value: number; onChange: (v: number) => void; step?: number; min?: number; max?: number; unit?: string }) {
  return (
    <label className="num-field">
      <span className="field-label">{label}</span>
      <span className="num-wrap">
      <input
        type="number"
        value={round(value)}
        step={step}
        min={min}
        max={max}
        onChange={(e) => {
          const v = Number(e.target.value);
          if (e.target.value !== "" && Number.isFinite(v)) onChange(min !== undefined ? Math.max(min, max !== undefined ? Math.min(max, v) : v) : v);
        }}
      />
      <em>{unit}</em>
      </span>
    </label>
  );
}

function PaperProps({ layout, onChange }: { layout: Layout; onChange: (l: Layout, key?: string) => void }) {
  const auto = layout.length === null;
  return (
    <div className="props-group">
      <h2>{t.customPaper}</h2>
      <Segmented<"auto" | "fixed">
        label={t.customLength}
        value={auto ? "auto" : "fixed"}
        onChange={(v) => onChange({ ...layout, length: v === "auto" ? null : paperLength(layout) })}
        options={[
          { value: "auto", label: t.customLengthAuto, tip: t.customLengthAutoTip },
          { value: "fixed", label: t.customLengthFixed },
        ]}
      />
      {!auto && <Num label={t.customLength} value={layout.length!} min={10} max={2000} step={5} onChange={(length) => onChange({ ...layout, length }, "length")} />}
      <ul className="hints small muted">
        {t.customHints.map((h) => (
          <li key={h}>{h}</li>
        ))}
      </ul>
    </div>
  );
}

function ItemProps({
  item,
  images,
  onPatch,
  onRemove,
  onDuplicate,
  onFront,
  onBack,
}: {
  item: Item;
  images: Map<string, HTMLImageElement>;
  onPatch: (p: Partial<Item>, key?: string) => void;
  onRemove: () => void;
  onDuplicate: () => void;
  onFront: () => void;
  onBack: () => void;
}) {
  const turn = (d: number) => onPatch({ rotate: ((((item.rotate + d) % 360) + 540) % 360) - 180 }, "turn");
  const aligns: [Align, typeof AlignLeft][] = [
    ["left", AlignLeft],
    ["center", AlignCenter],
    ["right", AlignRight],
  ];
  return (
    <div className="props-group">
      <h2>{t.customKinds[item.type === "line" && item.dashed ? "dashed" : item.type]}</h2>

      {item.type === "text" && (
        <>
          <textarea value={item.text} rows={3} onChange={(e) => onPatch({ text: e.target.value }, `text-${item.id}`)} aria-label={t.customTextField} />
          <div className="num-field">
            <span className="field-label">{t.font}</span>
            <FontPicker value={item.font} onChange={(font) => onPatch({ font })} />
          </div>
          <div className="geo">
            <Num label={t.size} unit="pt" value={item.size / PT} min={4} max={200} step={1} onChange={(v) => onPatch({ size: v * PT }, "size")} />
          </div>
          <div className="row icon-row">
            {(
              [
                ["bold", Bold, t.bold],
                ["italic", Italic, t.italic],
                ["underline", Underline, t.underline],
              ] as const
            ).map(([f, Icon, label]) => (
              <button key={f} type="button" className={`icon-btn${item[f] ? " on" : ""}`} aria-pressed={item[f]} title={label} aria-label={label} onClick={() => onPatch({ [f]: !item[f] })}>
                <Icon size={16} />
              </button>
            ))}
            <span className="divider" />
            {aligns.map(([a, Icon]) => (
              <button key={a} type="button" className={`icon-btn${item.align === a ? " on" : ""}`} aria-pressed={item.align === a} title={t.align[a]} aria-label={t.align[a]} onClick={() => onPatch({ align: a })}>
                <Icon size={16} />
              </button>
            ))}
          </div>
          <Toggle label={t.customInvert} checked={item.invert} onChange={(invert) => onPatch({ invert })} />
        </>
      )}

      {item.type === "image" && (
        <>
          <Range label={t.brightness} min={-60} max={60} value={item.brightness} onChange={(brightness) => onPatch({ brightness }, "brightness")} />
          <Range label={t.contrast} min={-60} max={60} value={item.contrast} onChange={(contrast) => onPatch({ contrast }, "contrast")} />
          <div className="row wrap">
            <Toggle label={t.invert} checked={item.invert} onChange={(invert) => onPatch({ invert })} />
            <button type="button" className={`icon-btn${item.flip ? " on" : ""}`} aria-pressed={item.flip} title={t.customFlip} aria-label={t.customFlip} onClick={() => onPatch({ flip: !item.flip })}>
              <FlipHorizontal2 size={16} />
            </button>
            <button
              type="button"
              className="btn ghost small"
              onClick={() => {
                const img = images.get(item.image);
                if (img) onPatch({ h: (item.w * img.height) / img.width });
              }}
            >
              {t.customRatio}
            </button>
          </div>
        </>
      )}

      {(item.type === "line" || item.type === "rect" || item.type === "ellipse") && (
        <>
          <Num
            label={t.customStroke}
            value={item.type === "line" ? item.h : item.stroke}
            min={0.2}
            max={item.type === "line" ? 10 : 8}
            step={0.1}
            onChange={(v) => (item.type === "line" ? onPatch({ h: v, stroke: v, y: item.y + (item.h - v) / 2 }, "stroke") : onPatch({ stroke: v }, "stroke"))}
          />
          <div className="row wrap">
            <Toggle label={t.customDashed} checked={item.dashed} onChange={(dashed) => onPatch({ dashed })} />
            {item.type !== "line" && <Toggle label={t.customFill} checked={item.fill} onChange={(fill) => onPatch({ fill })} />}
          </div>
          {item.type === "rect" && <Num label={t.customRadius} value={item.radius} min={0} max={50} step={0.5} onChange={(radius) => onPatch({ radius }, "radius")} />}
        </>
      )}

      {item.type === "line" ? (
        <LineGeo item={item} onPatch={onPatch} />
      ) : (
      <div className="geo">
        <Num label="X" value={item.x} onChange={(x) => onPatch({ x }, "x")} />
        <Num label="Y" value={item.y} onChange={(y) => onPatch({ y }, "y")} />
        <Num label={t.customW} value={item.w} min={MIN} onChange={(w) => onPatch({ w }, "w")} />
        {item.type !== "text" && <Num label={t.customH} value={item.h} min={MIN} onChange={(h) => onPatch({ h }, "h")} />}
      </div>
      )}
      <div className="row angle-row">
        <Num label={t.customAngle} unit="°" value={item.rotate} min={-180} max={180} step={1} onChange={(rotate) => onPatch({ rotate }, "rotate")} />
        <button type="button" className="icon-btn" onClick={() => turn(-90)} title={t.customTurnLeft} aria-label={t.customTurnLeft}>
          <RotateCcw size={16} />
        </button>
        <button type="button" className="icon-btn" onClick={() => turn(90)} title={t.customTurnRight} aria-label={t.customTurnRight}>
          <RotateCw size={16} />
        </button>
      </div>

      <div className="row wrap props-actions">
        <button type="button" className="icon-btn" onClick={onFront} title={t.customFront} aria-label={t.customFront}>
          <BringToFront size={16} />
        </button>
        <button type="button" className="icon-btn" onClick={onBack} title={t.customBack} aria-label={t.customBack}>
          <SendToBack size={16} />
        </button>
        <button type="button" className="icon-btn" onClick={onDuplicate} title={t.customDuplicate} aria-label={t.customDuplicate}>
          <Copy size={16} />
        </button>
        <span className="spacer" />
        <button type="button" className="btn ghost small danger" onClick={onRemove}>
          <Trash2 size={14} /> {t.remove}
        </button>
      </div>
    </div>
  );
}

/** A line's two ends, editable as coordinates. */
function LineGeo({ item, onPatch }: { item: ShapeItem; onPatch: (p: Partial<Item>, key?: string) => void }) {
  const [[x1, y1], [x2, y2]] = lineEnds(item);
  const set = (a: number, b: number, c: number, d: number, key: string) => {
    const { x, y, w, rotate } = lineBetween(item, a, b, c, d);
    onPatch({ x, y, w, rotate }, key);
  };
  return (
    <div className="geo">
      <Num label={t.customStart("X")} value={x1} onChange={(v) => set(v, y1, x2, y2, "x1")} />
      <Num label={t.customStart("Y")} value={y1} onChange={(v) => set(x1, v, x2, y2, "y1")} />
      <Num label={t.customEnd("X")} value={x2} onChange={(v) => set(x1, y1, v, y2, "x2")} />
      <Num label={t.customEnd("Y")} value={y2} onChange={(v) => set(x1, y1, x2, v, "y2")} />
    </div>
  );
}
