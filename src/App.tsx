import { useEffect, useMemo, useRef, useState, type ComponentType } from "react";
import { CheckSquare, FileText, Flag, Image as ImageIcon, LayoutList, StickyNote, Tag } from "lucide-react";
import { api, type Content, type ModelSummary, type QueueState, type Settings, type State } from "./api";
import { t } from "./i18n";
import { toPng } from "./lib/canvas";
import { PrinterPanel, type JobUi } from "./PrinterPanel";
import { SettingsMenu } from "./Settings";
import { Compose } from "./tools/Compose";
import { Document } from "./tools/Document";
import { Label } from "./tools/Label";
import { Photo } from "./tools/Photo";
import { Banner, Note, Todo } from "./tools/Templates";
import type { Output, ToolProps } from "./tools/ui";

type ToolId = keyof typeof t.tools;

const TOOLS: { id: ToolId; icon: typeof ImageIcon; Comp: ComponentType<ToolProps> }[] = [
  { id: "photo", icon: ImageIcon, Comp: Photo },
  { id: "compose", icon: LayoutList, Comp: Compose },
  { id: "document", icon: FileText, Comp: Document },
  { id: "label", icon: Tag, Comp: Label },
  { id: "note", icon: StickyNote, Comp: Note },
  { id: "todo", icon: CheckSquare, Comp: Todo },
  { id: "banner", icon: Flag, Comp: Banner },
];

const emptyState: State = { connecting: false, printer: null, status: null, busy: false, queue: 0 };
const emptyQueue: QueueState = { items: [], mode: { pause: true, separators: true }, running: false, waiting: false, current: null, job: null, error: null };

export default function App() {
  const [state, setState] = useState<State>(emptyState);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [models, setModels] = useState<ModelSummary[]>([]);
  const [previewModel, setPreviewModel] = useState("SC03h");
  const [tool, setTool] = useState<ToolId>("photo");
  const [outputs, setOutputs] = useState<Partial<Record<ToolId, Output>>>({});
  const [contentFor, setContentFor] = useState<Partial<Record<ToolId, Content>>>({});
  const [job, setJobBase] = useState<Omit<JobUi, "content">>({ gray: false, density: 4, copies: 1 });
  const [previews, setPreviews] = useState<string[]>([]);
  const [progress, setProgress] = useState<number | null>(null);
  const [message, setMessage] = useState<{ kind: "ok" | "error"; text: string } | null>(null);
  const currentJob = useRef<number | null>(null);
  const [queue, setQueue] = useState<QueueState>(emptyQueue);
  const [queueProgress, setQueueProgress] = useState<number | null>(null);
  const queueJob = useRef<number | null>(null);

  useEffect(() => {
    api.state().then(setState).catch(() => {});
    api.settings().then(setSettings).catch(() => {});
    api.models().then(setModels).catch(() => {});
    api.queue.state().then(setQueue).catch(() => {});
    return api.onEvent((e) => {
      if (e.type === "queue") {
        const { type: _ignored, ...q } = e;
        if (q.job !== queueJob.current) setQueueProgress(null);
        queueJob.current = q.job;
        setQueue(q);
      } else if (e.type === "progress" && e.job === queueJob.current) {
        setQueueProgress(e.total ? e.sent / e.total : 0);
      } else if (e.type === "state") {
        const { type: _ignored, ...s } = e;
        setState(s);
      } else if (e.type === "progress" && e.job === currentJob.current) {
        setProgress(e.total ? e.sent / e.total : 0);
      } else if (e.type === "jobDone" && e.job === currentJob.current) {
        currentJob.current = null;
        setProgress(null);
        const code = (e.error ?? "").replace(/^printer problem: /, "");
        setMessage(e.ok ? { kind: "ok", text: t.printed } : { kind: "error", text: t.printFailed(t.problems[code] ?? code) });
      }
    });
  }, []);

  // Without a printer, preview with the last used model.
  useEffect(() => {
    const last = settings?.lastName;
    if (!last || !models.length) return;
    const m = models.filter((x) => last.startsWith(x.modelNo)).sort((a, b) => b.modelNo.length - a.modelNo.length)[0];
    if (m) setPreviewModel(m.modelNo);
  }, [settings?.lastName, models.length]);

  const model = useMemo(() => {
    const no = state.printer?.modelNo ?? previewModel;
    return models.find((m) => m.modelNo === no);
  }, [state.printer?.modelNo, previewModel, models]);

  const ctx = useMemo(
    () => ({ width: state.printer?.widthPx ?? model?.widthPx ?? 384, dpi: state.printer?.dpi ?? model?.dpi ?? 203, labelCapable: state.printer?.label ?? model?.label ?? false }),
    [state.printer, model],
  );

  const out = outputs[tool];
  const content = contentFor[tool] ?? out?.content ?? "image";
  const grayCapable = state.printer?.gray ?? model?.gray ?? false;
  const fullJob: JobUi = { ...job, content };
  const options = {
    density: job.density,
    content,
    gray: job.gray && grayCapable,
    label: !!out?.label,
    labelHeightMm: out?.labelHeightMm,
    copies: job.copies,
  };

  const pngs = useMemo(() => (out?.pages ?? []).map(toPng), [out]);

  useEffect(() => {
    if (!pngs.length) return setPreviews([]);
    let live = true;
    const id = setTimeout(() => {
      api.preview(pngs, options, state.printer ? undefined : previewModel).then((p) => live && setPreviews(p), () => {});
    }, 220);
    return () => {
      live = false;
      clearTimeout(id);
    };
  }, [pngs, options.density, options.content, options.gray, state.printer?.address, previewModel, settings]);

  const print = async () => {
    setMessage(null);
    try {
      setProgress(0);
      currentJob.current = await api.print(pngs, options);
    } catch (e) {
      setProgress(null);
      setMessage({ kind: "error", text: t.printFailed(String(e)) });
    }
  };

  const addToQueue = async () => {
    setMessage(null);
    try {
      await api.queue.add(pngs, options, out?.title?.trim() || t.tools[tool], tool);
      const q = await api.queue.state();
      setQueue(q);
      setMessage({ kind: "ok", text: t.addedToQueue(q.items.reduce((n, i) => n + i.copies - i.done, 0)) });
    } catch (e) {
      setMessage({ kind: "error", text: t.printFailed(String(e)) });
    }
  };

  return (
    <div className="app">
      <nav className="rail" aria-label="Outils">
        <div className="brand" aria-hidden>
          <img src="./icon.png" alt="" />
        </div>
        {TOOLS.map(({ id, icon: Icon }) => (
          <button key={id} type="button" className={`rail-btn${tool === id ? " on" : ""}`} onClick={() => setTool(id)} aria-current={tool === id ? "page" : undefined}>
            <Icon size={20} strokeWidth={1.75} />
            <span>{t.tools[id]}</span>
          </button>
        ))}
        <span className="rail-spacer" />
        {api.canManage && <SettingsMenu />}
      </nav>
      <main className="editor">
        <h1>{t.tools[tool]}</h1>
        {TOOLS.map(({ id, Comp }) => (
          <section key={id} hidden={id !== tool} className="editor-body">
            <Comp ctx={ctx} onOutput={(o) => setOutputs((s) => ({ ...s, [id]: o }))} />
          </section>
        ))}
      </main>
      <PrinterPanel
        state={state}
        settings={settings}
        onSettings={setSettings}
        models={models}
        previewModel={previewModel}
        setPreviewModel={setPreviewModel}
        previews={previews}
        dpi={ctx.dpi}
        job={fullJob}
        setJob={(j) => {
          setContentFor((s) => ({ ...s, [tool]: j.content }));
          setJobBase({ gray: j.gray, density: j.density, copies: j.copies });
        }}
        grayCapable={grayCapable}
        canPrint={pngs.length > 0}
        progress={progress}
        onPrint={print}
        onQueue={addToQueue}
        queue={queue}
        queueProgress={queueProgress}
        message={message}
      />
    </div>
  );
}
