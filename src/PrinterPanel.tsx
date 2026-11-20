import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Copy, ListOrdered, ListPlus, QrCode, BatteryCharging, BatteryFull, BatteryLow, BatteryMedium, Bluetooth, Loader2, Printer, RefreshCw, SlidersHorizontal, X } from "lucide-react";
import { api, type ShareInfo, type Content, type ModelSummary, type PrinterInfo, type QueueState, type Seen, type Settings, type State, type Status } from "./api";
import { t } from "./i18n";
import { qrCanvas } from "./lib/canvas";
import { QueueControls, QueueList, sheetsLeft } from "./Queue";
import { Range, Segmented, Toggle } from "./tools/ui";

export interface JobUi {
  content: Content;
  gray: boolean;
  density: number;
  copies: number;
}

function problemOf(s: Status | null): string | null {
  if (!s) return null;
  if (s.noPaper) return "no_paper";
  if (s.coverOpen) return "cover_open";
  if (s.overheat) return "overheat";
  if (s.lowBattery) return "low_battery";
  return null;
}

function Battery({ s }: { s: Status | null }) {
  if (!s) return null;
  const pct = s.batteryPercent ?? (s.batteryLevel != null ? Math.round((s.batteryLevel / 6) * 100) : null);
  if (pct == null && !s.charging) return null;
  const Icon = s.charging ? BatteryCharging : pct == null ? BatteryMedium : pct < 25 ? BatteryLow : pct < 70 ? BatteryMedium : BatteryFull;
  return (
    <span className="battery" title={s.charging ? t.charging : `${t.battery} ${pct} %`}>
      <Icon size={18} />
      {pct != null && <span>{pct} %</span>}
    </span>
  );
}

function Connect({ settings, onClose }: { settings: Settings | null; onClose: () => void }) {
  const [list, setList] = useState<Seen[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const scan = () => {
    setList(null);
    setError(null);
    api.scan(6).then(setList, (e) => setError(String(e)));
  };
  useEffect(scan, []);
  const connect = (address: string, name?: string) => {
    setBusy(address);
    setError(null);
    api.connect(address, name).then(onClose, (e) => {
      setError(String(e));
      setBusy(null);
    });
  };
  return (
    <div className="popover" role="dialog" aria-label={t.connect}>
      <div className="popover-head">
        <strong>{t.connect}</strong>
        <button type="button" className="icon-btn" onClick={onClose} aria-label={t.close}>
          <X size={16} />
        </button>
      </div>
      {settings?.lastName && settings.lastAddress && !list?.some((s) => s.address === settings.lastAddress) && (
        <button type="button" className="device" onClick={() => connect(settings.lastAddress!, settings.lastName!)} disabled={!!busy}>
          <Bluetooth size={16} />
          <span className="grow">{t.reconnect(settings.lastName)}</span>
          {busy === settings.lastAddress && <Loader2 size={16} className="spin" />}
        </button>
      )}
      {list === null ? (
        <p className="muted row">
          <Loader2 size={16} className="spin" /> {t.searching}
        </p>
      ) : list.length === 0 ? (
        <p className="muted">{t.nothingFound}</p>
      ) : (
        list.map((s) => (
          <button key={s.address} type="button" className="device" onClick={() => connect(s.address)} disabled={!!busy}>
            <Printer size={16} />
            <span className="grow">
              <span className="device-name">{s.name}</span>
              <span className="muted small">
                {s.modelNo} · {s.paperMm} mm
              </span>
            </span>
            {busy === s.address ? <Loader2 size={16} className="spin" /> : <Signal rssi={s.rssi} />}
          </button>
        ))
      )}
      {error && <p className="error">{error}</p>}
      {list !== null && (
        <button type="button" className="btn ghost" onClick={scan}>
          <RefreshCw size={15} /> {t.searchAgain}
        </button>
      )}
    </div>
  );
}

function ShareRow({ settings, onSettings }: { settings: Settings | null; onSettings: (s: Settings) => void }) {
  const [info, setInfo] = useState<ShareInfo | null>(null);
  const [copied, setCopied] = useState(false);
  const [qr, setQr] = useState<string | null>(null);
  useEffect(() => {
    api.shareInfo().then(setInfo, () => {});
    return api.onShare(setInfo);
  }, []);
  const on = settings?.shareOnLan ?? true;
  const ipUrl = info?.urls[0];
  const main = info?.localUrl ?? ipUrl;
  const toggleQr = async () => {
    if (qr || !ipUrl) return setQr(null);
    const c = await qrCanvas(ipUrl, 220);
    setQr(c.toDataURL());
  };
  const tidy = (u: string) => u.replace(/\/$/, "");
  return (
    <div className="share">
      <Toggle label={t.share} tip={t.shareTip} checked={on} onChange={(v) => api.setShare(v).then(onSettings, () => {})} />
      {on && main && (
        <div className="share-url">
          <span className="muted small">{t.shareOpen}</span>
          <code>{tidy(main)}</code>
          <span className="row">
            <button
              type="button"
              className="icon-btn"
              title={copied ? t.copied : t.copy}
              aria-label={t.copy}
              onClick={() => navigator.clipboard.writeText(main).then(() => (setCopied(true), setTimeout(() => setCopied(false), 1500)))}
            >
              <Copy size={14} />
            </button>
            <button type="button" className={`icon-btn${qr ? " on" : ""}`} title={t.showQr} aria-label={t.showQr} onClick={toggleQr}>
              <QrCode size={14} />
            </button>
          </span>
          {info?.localUrl && ipUrl && (
            <span className="muted small">
              {t.orIp} {tidy(ipUrl)}
            </span>
          )}
          {qr && <img src={qr} alt={ipUrl} className="share-qr" />}
        </div>
      )}
      {on && info?.firewallBlocked && (
        <div className="problem row">
          <span className="grow">{t.firewallBlocked}</span>
          <button type="button" className="btn small" onClick={() => api.retryFirewall()}>
            {t.allowFirewall}
          </button>
        </div>
      )}
    </div>
  );
}

function Signal({ rssi }: { rssi: number | null }) {
  const bars = rssi == null ? 0 : rssi > -60 ? 3 : rssi > -75 ? 2 : 1;
  return (
    <span className="signal" title={rssi != null ? `${rssi} dBm` : undefined} aria-hidden>
      {[1, 2, 3].map((b) => (
        <i key={b} className={b <= bars ? "on" : ""} style={{ height: 4 + b * 4 }} />
      ))}
    </span>
  );
}

function Calibrate({ printer, settings, onClose, onSettings }: { printer: PrinterInfo; settings: Settings | null; onClose: () => void; onSettings: (s: Settings) => void }) {
  const cur = settings?.printers[printer.address]?.grayCurve;
  const [firstGray, setFirstGray] = useState(cur ? cur.lo + 2 : 5);
  const [firstBlack, setFirstBlack] = useState(cur ? cur.hi + 1 : 14);
  const [gamma, setGamma] = useState(cur?.gamma ?? 1);
  const [error, setError] = useState<string | null>(null);
  const base = settings?.printers[printer.address] ?? { grayCurve: null, density: null };
  const save = (grayCurve: typeof cur | null) =>
    api.setPrinterSettings(printer.address, { ...base, grayCurve: grayCurve ?? null }).then(
      (s) => (onSettings(s), onClose()),
      (e) => setError(String(e)),
    );
  return (
    <div className="modal" role="dialog" aria-modal onClick={onClose}>
      <div className="modal-box narrow" onClick={(e) => e.stopPropagation()}>
        <h2>{t.calibrate}</h2>
        <p className="muted">{t.calibrateIntro}</p>
        <button type="button" className="btn" onClick={() => api.printGrayWedge().catch((e) => setError(String(e)))}>
          <Printer size={16} /> {t.printWedge}
        </button>
        <Range label={t.firstGray} min={1} max={15} value={firstGray} onChange={(v) => (setFirstGray(v), v >= firstBlack && setFirstBlack(v + 1))} />
        <Range label={t.firstBlack} min={2} max={16} value={firstBlack} onChange={(v) => (setFirstBlack(v), v <= firstGray && setFirstGray(v - 1))} />
        <Range label={t.midtones} tip={t.midtonesTip} min={0.4} max={1.6} step={0.05} value={gamma} format={(v) => v.toFixed(2)} onChange={setGamma} />
        {error && <p className="error">{error}</p>}
        <div className="row end">
          <button type="button" className="btn ghost" onClick={() => save(null)}>
            {t.reset}
          </button>
          <button type="button" className="btn primary" onClick={() => save({ lo: Math.max(0, firstGray - 2), hi: firstBlack - 1, gamma })}>
            {t.save}
          </button>
        </div>
      </div>
    </div>
  );
}

export function PrinterPanel({
  state,
  settings,
  onSettings,
  models,
  previewModel,
  setPreviewModel,
  previews,
  dpi,
  job,
  setJob,
  grayCapable,
  canPrint,
  progress,
  onPrint,
  onQueue,
  addCount,
  queue,
  queueProgress,
  message,
  part,
}: {
  state: State;
  settings: Settings | null;
  onSettings: (s: Settings) => void;
  models: ModelSummary[];
  previewModel: string;
  setPreviewModel: (m: string) => void;
  previews: string[];
  dpi: number;
  job: JobUi;
  setJob: (j: JobUi) => void;
  grayCapable: boolean;
  canPrint: boolean;
  progress: number | null;
  onPrint: () => void;
  onQueue: () => void;
  /** Sheets that "Add to queue" would add. */
  addCount: number;
  queue: QueueState;
  queueProgress: number | null;
  message: { kind: "ok" | "error"; text: string } | null;
  /** On phones: the printer bar at the top, or the print tray at the bottom. */
  part?: "top" | "tray";
}) {
  const [connecting, setConnecting] = useState(false);
  const [trayOpen, setTrayOpen] = useState(false);
  const [calibrating, setCalibrating] = useState(false);
  const [mm, setMm] = useState(0);
  const [view, setView] = useState<"preview" | "queue">("preview");
  useEffect(() => {
    let live = true;
    Promise.all(previews.map((src) => new Promise<number>((res) => {
      const i = new Image();
      i.onload = () => res(i.naturalHeight);
      i.onerror = () => res(0);
      i.src = src;
    }))).then((hs) => live && setMm(Math.round((hs.reduce((a, b) => a + b, 0) / (dpi || 203)) * 25.4)));
    return () => {
      live = false;
    };
  }, [previews, dpi]);
  const p = state.printer;
  const problem = problemOf(state.status);
  const printing = state.busy || progress !== null;
  const blocked = problem !== null && problem !== "low_battery";
  const left = sheetsLeft(queue);

  const viewSwitch = (
    <Segmented<"preview" | "queue">
      label={t.panelView}
      value={view}
      onChange={setView}
      options={[
        { value: "preview", label: t.preview },
        {
          value: "queue",
          label: (
            <span className="row center">
              {t.queue}
              {queue.items.length > 0 && <span className={`count${queue.running ? " live" : ""}`}>{left}</span>}
            </span>
          ),
        },
      ]}
    />
  );

  const paper =
    view === "queue" ? (
      <QueueList queue={queue} progress={queueProgress} />
    ) : previews.length === 0 ? (
      <div className="ticket empty-ticket">
        <p>{t.emptyTicket}</p>
      </div>
    ) : (
      <div className={`ticket${printing ? " feeding" : ""}`}>
        {previews.map((src, i) => (
          <img key={i} src={src} alt="" />
        ))}
      </div>
    );

  const options = (
    <>
      <Segmented<Content>
        label={t.content}
        value={job.content}
        onChange={(content) => setJob({ ...job, content })}
        options={(["text", "image", "photo"] as Content[]).map((c) => ({ value: c, label: t.contentKinds[c], tip: t.contentTips[c] }))}
      />
      <div className="row wrap">
        {grayCapable && <Toggle label={t.gray} tip={t.grayTip} checked={job.gray} onChange={(gray) => setJob({ ...job, gray })} />}
        <label className="copies" title={t.copies}>
          {t.copies}
          <input type="number" min={1} max={50} value={job.copies} onChange={(e) => setJob({ ...job, copies: Math.max(1, Math.min(50, Number(e.target.value) || 1)) })} className="num" />
        </label>
      </div>
      <Range label={t.density} tip={t.densityTip} min={1} max={7} value={job.density} onChange={(density) => setJob({ ...job, density })} />
    </>
  );

  const printButton = (withLength: boolean) => (
    <button type="button" className="print-btn" onClick={onPrint} disabled={!canPrint || !p || printing || blocked}>
      {printing ? (
        <>
          <Loader2 size={18} className="spin" /> {t.printing}
          {progress !== null && <span className="pct">{Math.round(progress * 100)} %</span>}
        </>
      ) : (
        <>
          <Printer size={18} /> {t.print}
          {withLength && mm > 0 && previews.length > 0 && <span className="pct">{t.length(mm)}</span>}
        </>
      )}
      {progress !== null && <span className="bar" style={{ width: `${progress * 100}%` }} />}
    </button>
  );

  const addButton = (
    <button type="button" className="btn ghost" onClick={onQueue} disabled={!canPrint}>
      <ListPlus size={16} /> {addCount > 1 ? t.addSheets(addCount) : t.addToQueue}
    </button>
  );

  if (part === "top") {
    // Phones: the printer as a bar across the top; tapping it opens the printer sheet.
    return (
      <header className="m-top">
        <button type="button" className="m-device" onClick={() => setConnecting(true)} aria-haspopup="dialog">
          <span className={`led${p ? " on" : state.connecting ? " wait" : ""}`} aria-hidden />
          <span className="m-device-text">
            <span className="device-title">{p ? p.name : state.connecting ? t.connecting : t.noPrinter}</span>
            <span className="muted small">{p ? `${p.modelNo} · ${p.paperMm} mm` : api.canManage ? t.tapToConnect : t.previewFor(previewModel)}</span>
          </span>
        </button>
        <Battery s={state.status} />
        {problem && <span className="m-problem">{t.problems[problem]}</span>}
        {connecting && createPortal(
          <div className="sheet-backdrop" onClick={() => setConnecting(false)}>
            <div className="sheet" role="dialog" aria-modal aria-label={t.printerSheet} onClick={(e) => e.stopPropagation()}>
              <span className="sheet-grab" aria-hidden />
              {p ? (
                <>
                  <div className="device-head">
                    <span className="led on" aria-hidden />
                    <div className="grow">
                      <div className="device-title">{p.name}</div>
                      <div className="muted small">
                        {p.modelNo} · {p.paperMm} mm{p.firmware ? ` · ${t.firmware} ${p.firmware}` : ""}
                      </div>
                    </div>
                    <Battery s={state.status} />
                  </div>
                  {api.canManage && <ShareRow settings={settings} onSettings={onSettings} />}
                  <div className="row wrap">
                    {api.canManage && p.gray && (
                      <button type="button" className="btn" onClick={() => (setConnecting(false), setCalibrating(true))}>
                        <SlidersHorizontal size={16} /> {t.calibrate}
                      </button>
                    )}
                    {api.canManage && (
                      <button type="button" className="btn" onClick={() => (api.disconnect(), setConnecting(false))}>
                        {t.disconnect}
                      </button>
                    )}
                  </div>
                </>
              ) : api.canManage ? (
                <Connect settings={settings} onClose={() => setConnecting(false)} />
              ) : (
                <label className="field">
                  <span className="field-label">{t.previewModel}</span>
                  <select value={previewModel} onChange={(e) => setPreviewModel(e.target.value)}>
                    {models.map((m) => (
                      <option key={m.modelNo} value={m.modelNo}>
                        {m.modelNo} ({m.paperMm} mm)
                      </option>
                    ))}
                  </select>
                </label>
              )}
            </div>
          </div>,
          document.body,
        )}
        {calibrating && p && createPortal(<Calibrate printer={p} settings={settings} onSettings={onSettings} onClose={() => setCalibrating(false)} />, document.body)}
      </header>
    );
  }

  if (part === "tray") {
    // Phones: a print tray along the bottom edge; the ticket sticks out of it and the tray slides up for details.
    const toggle = () => setTrayOpen((o) => !o);
    const status = message?.text ?? (problem ? t.problems[problem] : !p && api.canManage ? t.noPrinter : null);
    return (
      <>
        {trayOpen && <div className="tray-backdrop" onClick={() => setTrayOpen(false)} aria-hidden />}
        <aside className={`tray${trayOpen ? " open" : ""}`} aria-label={t.printTray}>
          <button type="button" className="tray-grab" onClick={toggle} aria-expanded={trayOpen} aria-label={trayOpen ? t.close : t.showDetails}>
            <span />
          </button>
          {trayOpen && (
            <div className="tray-body">
              {viewSwitch}
              <div className="ticket-scroll">{paper}</div>
              <div className="job">
                {view === "queue" ? (
                  <QueueControls queue={queue} canPrint={!!p && !blocked} progress={queueProgress} />
                ) : (
                  <>
                    {options}
                    {addButton}
                  </>
                )}
              </div>
            </div>
          )}
          {!(trayOpen && view === "queue") && (
            <div className="tray-bar">
              <button type="button" className={`tray-ticket${previews.length ? "" : " empty"}${printing ? " feeding" : ""}`} onClick={toggle} aria-label={t.showDetails}>
                {previews[0] && <img src={previews[0]} alt="" />}
              </button>
              <button type="button" className="tray-info" onClick={toggle}>
                <strong>{mm > 0 && previews.length ? `${mm} mm` : t.nothingYet}</strong>
                {status && <span className={`small ${message?.kind === "error" || (problem && !message) ? "error" : message ? "ok" : "muted"}`}>{status}</span>}
              </button>
              <button
                type="button"
                className="icon-btn tray-queue"
                onClick={() => (setView("queue"), setTrayOpen(true))}
                aria-label={t.queue}
                title={t.queue}
              >
                <ListOrdered size={20} />
                {queue.items.length > 0 && <span className={`count${queue.running ? " live" : ""}`}>{left}</span>}
              </button>
              {printButton(false)}
            </div>
          )}
        </aside>
      </>
    );
  }

  return (
    <aside className="printer">
      <div className="device-body">
        <div className="device-head">
          <span className={`led${p ? " on" : state.connecting ? " wait" : ""}`} aria-hidden />
          <div className="grow">
            {p ? (
              <>
                <div className="device-title">{p.name}</div>
                <div className="muted small" title={p.firmware ? `${t.firmware} ${p.firmware}` : undefined}>
                  {p.modelNo} · {p.paperMm} mm
                </div>
              </>
            ) : (
              <>
                <div className="device-title">{state.connecting ? t.connecting : t.noPrinter}</div>
                <label className="muted small model-pick" title={t.previewModelTip}>
                  {t.previewModel}{" "}
                  <select value={previewModel} onChange={(e) => setPreviewModel(e.target.value)}>
                    {models.map((m) => (
                      <option key={m.modelNo} value={m.modelNo}>
                        {m.modelNo} ({m.paperMm} mm)
                      </option>
                    ))}
                  </select>
                </label>
              </>
            )}
          </div>
          <Battery s={state.status} />
          {api.canManage &&
            (p ? (
              <>
                {p.gray && (
                  <button type="button" className="icon-btn" onClick={() => setCalibrating(true)} title={t.calibrateTip} aria-label={t.calibrate}>
                    <SlidersHorizontal size={16} />
                  </button>
                )}
                <button type="button" className="btn ghost small" onClick={() => api.disconnect()}>
                  {t.disconnect}
                </button>
              </>
            ) : (
              <button type="button" className="btn primary small" onClick={() => setConnecting(true)} disabled={state.connecting}>
                <Bluetooth size={15} /> {t.connect}
              </button>
            ))}
        </div>
        {problem && <div className="problem">{t.problems[problem]}</div>}
        {p && api.canManage && <ShareRow settings={settings} onSettings={onSettings} />}
        <div className="views">{viewSwitch}</div>
        <div className="slot" aria-hidden />
        {connecting && <Connect settings={settings} onClose={() => setConnecting(false)} />}
      </div>

      <div className="ticket-scroll">{paper}</div>

      <div className="job">
        {view === "queue" ? (
          <QueueControls queue={queue} canPrint={!!p && !blocked} progress={queueProgress} />
        ) : (
          <>
        {options}
        {printButton(true)}
        {addButton}
        {state.queue > 0 && <p className="muted small">{t.queued(state.queue)}</p>}
        {message && <p className={message.kind === "error" ? "error" : "ok"}>{message.text}</p>}
          </>
        )}
      </div>
      {calibrating && p && <Calibrate printer={p} settings={settings} onSettings={onSettings} onClose={() => setCalibrating(false)} />}
    </aside>
  );
}
