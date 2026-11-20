import { ArrowDown, ArrowUp, Loader2, ListOrdered, Scissors, Square, Trash2 } from "lucide-react";
import { api, type QueueState } from "./api";
import { t } from "./i18n";
import { IconButton, Segmented, Toggle } from "./tools/ui";

export const sheetsLeft = (q: QueueState) => q.items.reduce((n, i) => n + i.copies - i.done, 0);

function problem(e: string) {
  const code = e.replace(/^printer problem: /, "");
  return t.problems[code] ?? code;
}

/** The planned sheets, drawn as paper: torn apart when the queue pauses, one strip otherwise. */
export function QueueList({ queue, progress }: { queue: QueueState; progress: number | null }) {
  if (!queue.items.length) {
    return (
      <div className="ticket empty-ticket">
        <p>{t.queueEmpty}</p>
      </div>
    );
  }
  const strip = !queue.mode.pause;
  const cls = `queue ${strip ? "strip" : "torn"}${strip && queue.mode.separators ? " dashed" : ""}`;
  return (
    <ol className={cls} aria-label={t.queue}>
      {queue.items.map((it, i) => {
        const now = queue.current === it.id;
        const belowCurrent = i === 1 && queue.current === queue.items[0].id;
        return (
          <li key={it.id} className={`sheet${now ? " now" : ""}`}>
            <div className="sheet-paper">
              {it.thumb && <img src={it.thumb} alt="" />}
              {now && progress !== null && <span className="sheet-bar" style={{ height: `${progress * 100}%` }} />}
            </div>
            <div className="sheet-info">
              <strong className="ellipsis" title={it.title}>
                {it.title}
              </strong>
              <span className="muted small">
                {it.lengthMm} mm {it.copies > 1 && t.sheetCopies(it.done, it.copies)}
              </span>
              {it.source !== "local" && <span className="muted small">{t.fromDevice(it.source)}</span>}
              <span className="sheet-tools">
                {now ? (
                  <Loader2 size={15} className="spin" aria-label={t.printing} />
                ) : (
                  <>
                    <IconButton label={t.moveUp} onClick={() => api.queue.move(it.id, i - 1)} disabled={i === 0 || belowCurrent}>
                      <ArrowUp size={14} />
                    </IconButton>
                    <IconButton label={t.moveDown} onClick={() => api.queue.move(it.id, i + 1)} disabled={i === queue.items.length - 1}>
                      <ArrowDown size={14} />
                    </IconButton>
                    <IconButton label={t.remove} onClick={() => api.queue.remove(it.id)}>
                      <Trash2 size={14} />
                    </IconButton>
                  </>
                )}
              </span>
            </div>
          </li>
        );
      })}
    </ol>
  );
}

/** Queue controls, in place of the print button while the queue is shown. */
export function QueueControls({ queue, canPrint, progress }: { queue: QueueState; canPrint: boolean; progress: number | null }) {
  const left = sheetsLeft(queue);
  const mode = queue.mode;
  const setMode = (m: Partial<typeof mode>) => api.queue.mode({ ...mode, ...m });
  return (
    <>
      <Segmented<"pause" | "strip">
        label={t.queue}
        value={mode.pause ? "pause" : "strip"}
        onChange={(v) => setMode({ pause: v === "pause" })}
        options={[
          { value: "pause", label: t.queuePause, tip: t.queuePauseTip },
          { value: "strip", label: t.queueContinuous, tip: t.queueContinuousTip },
        ]}
      />
      {!mode.pause && <Toggle label={t.queueSeparators} tip={t.queueSeparatorsTip} checked={mode.separators} onChange={(separators) => setMode({ separators })} />}
      {queue.waiting ? (
        <>
          <p className="tear">
            <Scissors size={16} /> {t.tearOff}
          </p>
          <button type="button" className="print-btn" onClick={() => api.queue.next()} autoFocus>
            <ListOrdered size={18} /> {t.nextSheet}
          </button>
        </>
      ) : queue.running ? (
        <button type="button" className="print-btn" disabled>
          <Loader2 size={18} className="spin" /> {t.printing}
          {progress !== null && <span className="pct">{Math.round(progress * 100)} %</span>}
          {progress !== null && <span className="bar" style={{ width: `${progress * 100}%` }} />}
        </button>
      ) : (
        <button type="button" className="print-btn" onClick={() => api.queue.start()} disabled={!canPrint || left === 0}>
          <ListOrdered size={18} /> {t.printQueue(left)}
        </button>
      )}
      <div className="row">
        {queue.running ? (
          <button type="button" className="btn ghost small" onClick={() => api.queue.stop()}>
            <Square size={13} /> {t.stop}
          </button>
        ) : (
          queue.items.length > 0 && (
            <button type="button" className="btn ghost small" onClick={() => api.queue.clear()}>
              <Trash2 size={13} /> {t.clearQueue}
            </button>
          )
        )}
      </div>
      {queue.error && <p className="error">{t.printFailed(problem(queue.error))}</p>}
    </>
  );
}
