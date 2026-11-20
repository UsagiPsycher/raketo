import { useEffect, useRef, useState } from "react";
import { Camera, ImagePlus, RotateCw, Trash2 } from "lucide-react";
import { t } from "../i18n";
import { fileToDataUrl, fitImage, loadImage, noAdjust, type ImageAdjust } from "../lib/canvas";
import { Check, Empty, IconButton, Range, Toggle, type ToolProps } from "./ui";

interface Item {
  id: number;
  img: HTMLImageElement;
  name: string;
  adjust: ImageAdjust;
  /** Printed and added to the queue. */
  on: boolean;
}

let nextId = 1;

type Named = { img: HTMLImageElement; name: string };

async function namedImages(files: Iterable<File>): Promise<Named[]> {
  const out: Named[] = [];
  for (const f of files) {
    if (!f.type.startsWith("image/")) continue;
    out.push({ img: await loadImage(await fileToDataUrl(f)), name: f.name.replace(/\.[^.]+$/, "") });
  }
  return out;
}

export async function imagesFromFiles(files: Iterable<File>): Promise<HTMLImageElement[]> {
  return (await namedImages(files)).map((n) => n.img);
}

export function Photo({ ctx, onOutput }: ToolProps) {
  const [items, setItems] = useState<Item[]>([]);
  const [cam, setCam] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const camera = useRef<HTMLInputElement>(null);
  // Touch devices take pictures with the system camera instead of a live view.
  const touch = window.matchMedia("(pointer: coarse)").matches;
  const root = useRef<HTMLDivElement>(null);

  const add = (imgs: Named[]) =>
    setItems((s) => [...s, ...imgs.map(({ img, name }) => ({ id: nextId++, img, name: name || `${t.webcam} ${new Date().toLocaleTimeString().slice(0, 5)}`, adjust: { ...noAdjust }, on: true }))]);

  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      // Only while this tool is shown: others take pasted pictures too.
      if (!root.current?.offsetParent) return;
      const files = [...(e.clipboardData?.files ?? [])];
      if (files.length) namedImages(files).then(add);
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);

  useEffect(() => {
    const chosen = items.filter((i) => i.on).map((i) => ({ pages: [fitImage(i.img, ctx.width, i.adjust)], title: i.name }));
    onOutput({ pages: chosen.flatMap((c) => c.pages), content: "photo", title: chosen.length === 1 ? chosen[0].title : undefined, sheets: chosen });
  }, [items, ctx.width]);

  const update = (id: number, a: Partial<ImageAdjust>) =>
    setItems((s) => s.map((i) => (i.id === id ? { ...i, adjust: { ...i.adjust, ...a } } : i)));
  const pick = (id: number | null, on: boolean) => setItems((s) => s.map((i) => (id === null || i.id === id ? { ...i, on } : i)));
  const chosen = items.filter((i) => i.on).length;

  return (
    <div
      ref={root}
      className="tool"
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        namedImages(e.dataTransfer.files).then(add);
      }}
    >
      <input
        ref={input}
        type="file"
        accept="image/*"
        multiple
        hidden
        onChange={(e) => {
          // Copied first: clearing the input empties its live file list.
          if (e.target.files) namedImages([...e.target.files]).then(add);
          e.target.value = "";
        }}
      />
      <div className="toolbar">
        <button type="button" className="btn" onClick={() => input.current?.click()}>
          <ImagePlus size={16} /> {t.openImages}
        </button>
        <input
          ref={camera}
          type="file"
          accept="image/*"
          capture="environment"
          hidden
          onChange={(e) => {
            if (e.target.files) namedImages([...e.target.files]).then(add);
            e.target.value = "";
          }}
        />
        <button type="button" className="btn" onClick={() => (touch ? camera.current?.click() : setCam(true))}>
          <Camera size={16} /> {touch ? t.takePhoto : t.webcam}
        </button>
        {items.length > 1 && (
          <>
            <span className="spacer" />
            <Check label={t.photosChosen(chosen, items.length)} checked={chosen === items.length} mixed={chosen > 0 && chosen < items.length} onChange={(v) => pick(null, v)} />
          </>
        )}
      </div>
      {items.length === 0 ? (
        <Empty>
          {t.dropHere}{" "}
          <button type="button" className="link" onClick={() => input.current?.click()}>
            {t.openImages.toLowerCase()}
          </button>
          .
        </Empty>
      ) : (
        <ul className="cards">
          {items.map((it) => (
            <li key={it.id} className={`card${it.on ? "" : " off"}`}>
              <div className="card-pick">
                <img src={it.img.src} alt="" className="thumb" style={{ transform: `rotate(${it.adjust.rotate}deg)` }} onClick={() => pick(it.id, !it.on)} />
                <Check label={t.includePhoto(it.name)} hideLabel checked={it.on} onChange={(v) => pick(it.id, v)} />
              </div>
              <div className="card-body">
                <strong className="ellipsis card-title" title={it.name}>
                  {it.name}
                </strong>
                <Range label={t.brightness} min={-60} max={60} value={it.adjust.brightness} onChange={(v) => update(it.id, { brightness: v })} />
                <Range label={t.contrast} min={-60} max={60} value={it.adjust.contrast} onChange={(v) => update(it.id, { contrast: v })} />
                <div className="row">
                  <Toggle label={t.invert} checked={it.adjust.invert} onChange={(v) => update(it.id, { invert: v })} />
                  <span className="spacer" />
                  <IconButton label={t.rotate} onClick={() => update(it.id, { rotate: (((it.adjust.rotate + 90) % 360) as ImageAdjust["rotate"]) })}>
                    <RotateCw size={16} />
                  </IconButton>
                  <IconButton label={t.remove} onClick={() => setItems((s) => s.filter((x) => x.id !== it.id))}>
                    <Trash2 size={16} />
                  </IconButton>
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}
      {cam && <Webcam onClose={() => setCam(false)} onShot={(img) => { add([{ img, name: "" }]); setCam(false); }} />}
    </div>
  );
}

function Webcam({ onClose, onShot }: { onClose: () => void; onShot: (img: HTMLImageElement) => void }) {
  const video = useRef<HTMLVideoElement>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let stream: MediaStream | null = null;
    navigator.mediaDevices
      ?.getUserMedia({ video: true })
      .then((s) => {
        stream = s;
        if (video.current) video.current.srcObject = s;
      })
      .catch((e) => setError(String(e?.message ?? e)));
    return () => stream?.getTracks().forEach((tr) => tr.stop());
  }, []);
  const shoot = async () => {
    const v = video.current;
    if (!v) return;
    const c = document.createElement("canvas");
    c.width = v.videoWidth;
    c.height = v.videoHeight;
    c.getContext("2d")!.drawImage(v, 0, 0);
    onShot(await loadImage(c.toDataURL("image/png")));
  };
  return (
    <div className="modal" role="dialog" aria-modal onClick={onClose}>
      <div className="modal-box" onClick={(e) => e.stopPropagation()}>
        {error ? <p>{error}</p> : <video ref={video} autoPlay playsInline className="webcam" />}
        <div className="row end">
          <button type="button" className="btn" onClick={onClose}>
            {t.close}
          </button>
          <button type="button" className="btn primary" onClick={shoot} disabled={!!error}>
            <Camera size={16} /> {t.capture}
          </button>
        </div>
      </div>
    </div>
  );
}
