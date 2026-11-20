import { useState, type DragEvent } from "react";
import { GripVertical, ListOrdered, Loader2, Printer, Scissors, Square, Trash2 } from "lucide-react";
import { api, type QueueEntry, type QueueState } from "./api";
import { t } from "./i18n";
import { Check, IconButton, Toggle } from "./tools/ui";

const left = (i: QueueEntry) => i.copies - i.done;
/** Sheets that the next run will print: checked ones only. */
export const sheetsLeft = (q: QueueState) => q.items.reduce((n, i) => n + (i.selected ? left(i) : 0), 0);
const allSheets = (q: QueueState) => q.items.reduce((n, i) => n + left(i), 0);

function problem(e: string) {
  const code = e.replace(/^printer problem: /, "");
  return t.problems[code] ?? code;
}

/** The planned sheets, drawn as paper: torn apart when the queue pauses, one strip otherwise. */
export function QueueList({ queue, progress }: { queue: QueueState; progress: number | null }) {
  const [drag, setDrag] = useState<number | null>(null);
  // Drop position: the sheet is inserted before this index.
  const [over, setOver] = useState<number | null>(null);

  if (!queue.items.length) {
    return (
      <div className="queue-empty">
        <div className="queue-empty-art" aria-hidden>
          <span />
          <span />
          <span />
        </div>
        <p>{t.queueEmpty}</p>
      </div>
    );
  }

  const chosen = queue.items.filter((i) => i.selected);
  const mm = chosen.reduce((n, i) => n + i.lengthMm * left(i), 0);
  const strip = !queue.mode.pause;
  const cls = `queue ${strip ? "strip" : "torn"}${strip && queue.mode.separators ? " dashed" : ""}`;
  // The sheet being printed stays where it is.
  const locked = (it: QueueEntry) => queue.current === it.id;
  let order = 0;

  const move = (id: number, from: number, before: number) => {
    const to = from < before ? before - 1 : before;
    if (to !== from) api.queue.move(id, to);
  };
  const onOver = (e: DragEvent, i: number) => {
    if (drag === null) return;
    e.preventDefault();
    const r = e.currentTarget.getBoundingClientRect();
    setOver(e.clientY < r.top + r.height / 2 ? i : i + 1);
  };
  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    const from = queue.items.findIndex((x) => x.id === drag);
    if (drag !== null && over !== null && from >= 0) move(drag, from, over);
    setDrag(null);
    setOver(null);
  };

  return (
    <div className="queue-wrap">
      <div className="queue-head">
        <Check
          label={t.queueChosen(sheetsLeft(queue), allSheets(queue))}
          checked={chosen.length === queue.items.length}
          mixed={chosen.length > 0 && chosen.length < queue.items.length}
          onChange={(v) => api.queue.select(null, v)}
        />
        {mm > 0 && <span className="muted small">{t.length(mm)}</span>}
      </div>
      <ol className={cls} aria-label={t.queue} onDragLeave={(e) => e.currentTarget === e.target && setOver(null)}>
        {queue.items.map((it, i) => {
          const now = locked(it);
          const n = it.selected ? ++order : null;
          return (
            <li
              key={it.id}
              className={`sheet${now ? " now" : ""}${it.selected ? "" : " off"}${it.copies - it.done > 1 ? " stack" : ""}${drag === it.id ? " dragging" : ""}${over === i ? " drop-before" : ""}${over === i + 1 && i === queue.items.length - 1 ? " drop-after" : ""}`}
              draggable={!now}
              onDragStart={(e) => {
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", String(it.id));
                setDrag(it.id);
              }}
              onDragEnd={() => (setDrag(null), setOver(null))}
              onDragOver={(e) => onOver(e, i)}
              onDrop={onDrop}
            >
              <div className="sheet-check">
                {now ? <Loader2 size={16} className="spin" aria-label={t.printing} /> : <Check label={t.includeSheet(it.title)} hideLabel checked={it.selected} onChange={(v) => api.queue.select(it.id, v)} />}
              </div>
              <div className="sheet-paper" onClick={() => !now && api.queue.select(it.id, !it.selected)}>
                {it.thumb && <img src={it.thumb} alt="" draggable={false} />}
                {n !== null && <span className="sheet-no">{n}</span>}
                {now && progress !== null && <span className="sheet-bar" style={{ height: `${progress * 100}%` }} />}
              </div>
              <div className="sheet-info">
                <strong className="sheet-title" title={it.title}>
                  {it.title}
                </strong>
                <span className="sheet-meta muted small">
                  <span>{it.lengthMm} mm</span>
                  {it.copies > 1 && <span>{t.sheetCopies(it.done, it.copies)}</span>}
                </span>
                {it.source !== "local" && <span className="muted small">{t.fromDevice(it.source)}</span>}
                {!it.selected && <span className="sheet-skip small">{t.skipped}</span>}
              </div>
              {!now && (
                <div className="sheet-tools">
                  <button
                    type="button"
                    className="icon-btn grip"
                    aria-label={t.moveHint}
                    title={t.moveHint}
                    onKeyDown={(e) => {
                      if (e.key === "ArrowUp" && i > 0) (e.preventDefault(), api.queue.move(it.id, i - 1));
                      if (e.key === "ArrowDown" && i < queue.items.length - 1) (e.preventDefault(), api.queue.move(it.id, i + 1));
                    }}
                  >
                    <GripVertical size={15} />
                  </button>
                  <IconButton label={t.remove} onClick={() => api.queue.remove(it.id)}>
                    <Trash2 size={14} />
                  </IconButton>
                </div>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}

/** Small drawings of the two ways sheets come out. */
function ModeArt({ pause, dashed }: { pause: boolean; dashed: boolean }) {
  return pause ? (
    <span className="mode-art torn" aria-hidden>
      <i />
      <i />
    </span>
  ) : (
    <span className={`mode-art strip${dashed ? " dashed" : ""}`} aria-hidden>
      <i />
    </span>
  );
}

/** Queue controls, in place of the print button while the queue is shown. */
export function QueueControls({ queue, canPrint, progress }: { queue: QueueState; canPrint: boolean; progress: number | null }) {
  const todo = sheetsLeft(queue);
  const all = allSheets(queue);
  const total = queue.printed + todo;
  const mode = queue.mode;
  const setMode = (m: Partial<typeof mode>) => api.queue.mode({ ...mode, ...m });
  const modes = [
    { pause: true, label: t.queuePause, tip: t.queuePauseTip },
    { pause: false, label: t.queueContinuous, tip: t.queueContinuousTip },
  ];
  return (
    <>
      <div className="modes" role="radiogroup" aria-label={t.queueMode}>
        {modes.map((m) => (
          <button key={m.label} type="button" role="radio" aria-checked={mode.pause === m.pause} className={`mode${mode.pause === m.pause ? " on" : ""}`} onClick={() => setMode({ pause: m.pause })}>
            <ModeArt pause={m.pause} dashed={mode.separators} />
            <span className="mode-text">
              <strong>{m.label}</strong>
              <span className="small muted">{m.tip}</span>
            </span>
          </button>
        ))}
      </div>
      {!mode.pause && <Toggle label={t.queueSeparators} tip={t.queueSeparatorsTip} checked={mode.separators} onChange={(separators) => setMode({ separators })} />}

      {queue.waiting && (
        <div className="callout" role="status">
          <Scissors size={18} />
          <span>{t.tearOff(queue.printed)}</span>
        </div>
      )}

      {queue.waiting ? (
        <button type="button" className="print-btn" onClick={() => api.queue.next()} autoFocus>
          <Printer size={18} /> {t.nextSheet(queue.printed + 1, total)}
        </button>
      ) : queue.running ? (
        <button type="button" className="print-btn" disabled>
          <Loader2 size={18} className="spin" /> {t.sheetOf(queue.printed + 1, total)}
          {progress !== null && <span className="pct">{Math.round(progress * 100)} %</span>}
          {progress !== null && <span className="bar" style={{ width: `${progress * 100}%` }} />}
        </button>
      ) : (
        <button type="button" className="print-btn" onClick={() => api.queue.start()} disabled={!canPrint || todo === 0}>
          <ListOrdered size={18} /> {todo === 0 ? t.nothingChosen : todo === all ? t.printAll(all) : t.printSome(todo, all)}
        </button>
      )}

      <div className="row">
        {queue.running ? (
          <button type="button" className="btn ghost small" onClick={() => api.queue.stop()}>
            <Square size={13} /> {queue.waiting ? t.stop : t.stopAfter}
          </button>
        ) : (
          queue.items.length > 0 && (
            <button type="button" className="btn ghost small" onClick={() => api.queue.clear()}>
              <Trash2 size={13} /> {t.clearQueue}
            </button>
          )
        )}
        {!queue.running && !canPrint && queue.items.length > 0 && <span className="muted small">{t.connectToPrint}</span>}
      </div>
      {queue.error && <p className="error">{t.printFailed(problem(queue.error))}</p>}
    </>
  );
}
