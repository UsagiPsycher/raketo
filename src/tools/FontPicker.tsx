import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, Search } from "lucide-react";
import { api } from "../api";
import { t } from "../i18n";
import { FONTS } from "../lib/canvas";
import { fontFamily } from "../lib/rkt";

// Listed once per session: reading the system fonts takes a moment.
let cache: Promise<string[]> | null = null;
const systemFonts = () => (cache ??= api.systemFonts().catch(() => []));

const label = (font: string) => FONTS.find((f) => f.id === font)?.label ?? font;

/** Picks a built-in font or any font installed on the computer, each shown in its own face. */
export function FontPicker({ value, onChange }: { value: string; onChange: (font: string) => void }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [installed, setInstalled] = useState<string[]>([]);
  const [active, setActive] = useState(0);
  const box = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    if (open) systemFonts().then(setInstalled);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [open]);

  const options = useMemo(() => {
    const q = query.trim().toLowerCase();
    const all = [...FONTS.map((f) => ({ font: f.id as string, group: "app" })), ...installed.map((font) => ({ font, group: "system" }))];
    return q ? all.filter((o) => label(o.font).toLowerCase().includes(q)) : all;
  }, [query, installed]);

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const pick = (font: string) => {
    onChange(font);
    setOpen(false);
    setQuery("");
  };

  return (
    <div className="font-picker" ref={box}>
      <button type="button" className="font-current" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="listbox" title={label(value)}>
        <span style={{ fontFamily: fontFamily(value) }}>{label(value)}</span>
        <ChevronDown size={14} />
      </button>
      {open && (
        <div className="font-pop">
          <label className="font-search">
            <Search size={14} />
            <input
              autoFocus
              value={query}
              placeholder={t.fontSearch}
              aria-label={t.fontSearch}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown") (e.preventDefault(), setActive((a) => Math.min(options.length - 1, a + 1)));
                else if (e.key === "ArrowUp") (e.preventDefault(), setActive((a) => Math.max(0, a - 1)));
                else if (e.key === "Enter" && options[active]) (e.preventDefault(), pick(options[active].font));
                else if (e.key === "Escape") (e.stopPropagation(), setOpen(false));
              }}
            />
          </label>
          <ul className="font-list" role="listbox" ref={list} aria-label={t.font}>
            {options.map((o, i) => (
              <li key={`${o.group}-${o.font}`}>
                {(i === 0 || options[i - 1].group !== o.group) && <span className="font-group">{o.group === "app" ? t.fontsApp : t.fontsSystem}</span>}
                <button
                  type="button"
                  role="option"
                  data-i={i}
                  aria-selected={o.font === value}
                  className={`${i === active ? "active" : ""}${o.font === value ? " current" : ""}`}
                  style={{ fontFamily: fontFamily(o.font) }}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => pick(o.font)}
                >
                  {label(o.font)}
                </button>
              </li>
            ))}
            {!options.length && <li className="muted small font-none">{t.fontNone}</li>}
          </ul>
          {api.canAskFonts && !installed.length && (
            <button type="button" className="btn ghost small" onClick={() => api.systemFonts(true).then((f) => ((cache = Promise.resolve(f)), setInstalled(f)), () => {})}>
              {t.fontsAsk}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
