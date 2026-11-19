import { useEffect, useRef, useState } from "react";
import { Camera, ImagePlus, RotateCw, Trash2 } from "lucide-react";
import { t } from "../i18n";
import { fileToDataUrl, fitImage, loadImage, noAdjust, type ImageAdjust } from "../lib/canvas";
import { Empty, IconButton, Range, Toggle, type ToolProps } from "./ui";

interface Item {
  id: number;
  img: HTMLImageElement;
  adjust: ImageAdjust;
}

let nextId = 1;

export async function imagesFromFiles(files: Iterable<File>): Promise<HTMLImageElement[]> {
  const out: HTMLImageElement[] = [];
  for (const f of files) {
    if (!f.type.startsWith("image/")) continue;
    out.push(await loadImage(await fileToDataUrl(f)));
  }
  return out;
}

export function Photo({ ctx, onOutput }: ToolProps) {
  const [items, setItems] = useState<Item[]>([]);
  const [cam, setCam] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  const add = (imgs: HTMLImageElement[]) =>
    setItems((s) => [...s, ...imgs.map((img) => ({ id: nextId++, img, adjust: { ...noAdjust } }))]);

  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const files = [...(e.clipboardData?.files ?? [])];
      if (files.length) imagesFromFiles(files).then(add);
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);

  useEffect(() => {
    onOutput({ pages: items.map((i) => fitImage(i.img, ctx.width, i.adjust)), content: "photo" });
  }, [items, ctx.width]);

  const update = (id: number, a: Partial<ImageAdjust>) =>
    setItems((s) => s.map((i) => (i.id === id ? { ...i, adjust: { ...i.adjust, ...a } } : i)));

  return (
    <div
      className="tool"
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        imagesFromFiles(e.dataTransfer.files).then(add);
      }}
    >
      <input ref={input} type="file" accept="image/*" multiple hidden onChange={(e) => e.target.files && imagesFromFiles(e.target.files).then(add)} />
      <div className="toolbar">
        <button type="button" className="btn" onClick={() => input.current?.click()}>
          <ImagePlus size={16} /> {t.openImages}
        </button>
        <button type="button" className="btn" onClick={() => setCam(true)}>
          <Camera size={16} /> {t.webcam}
        </button>
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
            <li key={it.id} className="card">
              <img src={it.img.src} alt="" className="thumb" style={{ transform: `rotate(${it.adjust.rotate}deg)` }} />
              <div className="card-body">
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
      {cam && <Webcam onClose={() => setCam(false)} onShot={(img) => { add([img]); setCam(false); }} />}
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
