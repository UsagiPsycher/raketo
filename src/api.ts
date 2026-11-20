// One API for both front ends: the desktop app talks to Rust through Tauri,
// other computers on the network reach the same service over HTTP.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const inTauri = "__TAURI_INTERNALS__" in window;

export type Content = "text" | "image" | "photo";

export interface GrayCurve {
  lo: number;
  hi: number;
  gamma: number;
}

export interface JobOptions {
  density: number;
  content: Content;
  gray: boolean;
  label: boolean;
  copies: number;
  extraFeed?: number;
  grayCurve?: GrayCurve | null;
  labelHeightMm?: number;
}

export interface Status {
  noPaper: boolean;
  coverOpen: boolean;
  overheat: boolean;
  lowBattery: boolean;
  charging: boolean;
  printing: boolean;
  batteryPercent: number | null;
  batteryLevel: number | null;
}

export interface PrinterInfo {
  name: string;
  address: string;
  modelNo: string;
  firmware: string | null;
  status: Status | null;
  transport: "spp" | "ble";
  paperMm: number;
  widthPx: number;
  dpi: number;
  gray: boolean;
  label: boolean;
}

export interface State {
  connecting: boolean;
  printer: PrinterInfo | null;
  status: Status | null;
  busy: boolean;
  queue: number;
}

export interface Seen {
  name: string;
  address: string;
  rssi: number | null;
  modelNo: string;
  paperMm: number;
  transport: "spp" | "ble";
}

export interface ModelSummary {
  modelNo: string;
  paperMm: number;
  widthPx: number;
  dpi: number;
  gray: boolean;
  label: boolean;
  a4: boolean;
}

export interface PrinterSettings {
  grayCurve: GrayCurve | null;
  density: number | null;
  labelBaseline?: number | null;
}

export interface Settings {
  lastAddress: string | null;
  lastName: string | null;
  printers: Record<string, PrinterSettings>;
  shareOnLan: boolean;
  lanPort: number | null;
  lanName: string | null;
  queue: QueueMode;
}

export interface QueueMode {
  /** Wait for the user to tear off each sheet before the next one. */
  pause: boolean;
  /** Without pauses, print a dashed cut line between sheets. */
  separators: boolean;
}

export interface QueueEntry {
  id: number;
  title: string;
  kind: string;
  thumb: string;
  copies: number;
  done: number;
  lengthMm: number;
  label: boolean;
  source: string;
}

export interface QueueState {
  items: QueueEntry[];
  mode: QueueMode;
  running: boolean;
  waiting: boolean;
  current: number | null;
  job: number | null;
  error: string | null;
}

export interface ShareInfo {
  port: number;
  name: string;
  urls: string[];
  ipp: string[];
  localUrl: string | null;
  firewallBlocked: boolean;
}

export type ServiceEvent =
  | ({ type: "state" } & State)
  | { type: "progress"; job: number; sent: number; total: number }
  | { type: "jobDone"; job: number; ok: boolean; error: string | null }
  | ({ type: "queue" } & QueueState);

async function http<T>(path: string, body?: unknown): Promise<T> {
  const r = await fetch(`./api/${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await r.text();
  if (!r.ok) throw new Error(text || r.statusText);
  return (text ? JSON.parse(text) : undefined) as T;
}

function call<T>(cmd: string, args: Record<string, unknown> = {}, httpPath?: string, get = false): Promise<T> {
  if (inTauri) return invoke<T>(cmd, args);
  return http<T>(httpPath ?? cmd, get ? undefined : args);
}

export const api = {
  /** The web page can only print; the printer is managed on the host computer. */
  canManage: inTauri,
  models: () => call<ModelSummary[]>("models", {}, "models", true),
  scan: (seconds = 6) => call<Seen[]>("scan", { seconds }),
  connect: (address: string, name?: string) => call<State>("connect", { address, name }),
  reconnect: () => call<State>("reconnect"),
  disconnect: () => call<State>("disconnect"),
  state: () => call<State>("state", {}, "state", true),
  settings: () => call<Settings>("settings", {}, "settings", true),
  setPrinterSettings: (address: string, value: PrinterSettings) =>
    call<Settings>("set_printer_settings", { address, value }),
  preview: (pages: string[], options: JobOptions, modelNo?: string) =>
    call<string[]>("preview", { pages, options, modelNo }),
  print: (pages: string[], options: JobOptions) => call<number>("print", { pages, options }),
  printGrayWedge: () => call<number>("print_gray_wedge"),
  queue: {
    state: () => call<QueueState>("queue_state", {}, "queue", true),
    add: (pages: string[], options: JobOptions, title: string, kind: string) =>
      call<number>("queue_add", { pages, options, title, kind }, "queue/add"),
    remove: (id: number) => call<QueueState>("queue_remove", { id }, "queue/remove"),
    move: (id: number, index: number) => call<QueueState>("queue_move", { id, index }, "queue/move"),
    clear: () => call<QueueState>("queue_clear", {}, "queue/clear"),
    mode: (mode: QueueMode) => call<QueueState>("queue_mode", { mode }, "queue/mode"),
    start: () => call<QueueState>("queue_start", {}, "queue/start"),
    next: () => call<QueueState>("queue_continue", {}, "queue/continue"),
    stop: () => call<QueueState>("queue_stop", {}, "queue/stop"),
  },
  shareInfo: () => (inTauri ? invoke<ShareInfo | null>("share_info") : Promise.resolve(null)),
  setShare: (enabled: boolean) => invoke<Settings>("set_share", { enabled }),
  retryFirewall: () => invoke<void>("retry_firewall"),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  onShare(cb: (s: ShareInfo | null) => void): () => void {
    if (!inTauri) return () => {};
    const un = listen<ShareInfo | null>("share", (e) => cb(e.payload));
    return () => void un.then((f) => f());
  },
  onEvent(cb: (e: ServiceEvent) => void): () => void {
    if (inTauri) {
      const un = listen<ServiceEvent>("service", (e) => cb(e.payload));
      return () => void un.then((f) => f());
    }
    const es = new EventSource("./api/events");
    es.onmessage = (m) => cb(JSON.parse(m.data));
    return () => es.close();
  },
};
