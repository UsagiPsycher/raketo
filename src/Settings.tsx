import { useEffect, useRef, useState } from "react";
import { Settings as Gear } from "lucide-react";
import { api } from "./api";
import { t } from "./i18n";
import { Toggle } from "./tools/ui";

/** App settings, desktop only: things that belong to this computer. */
export function SettingsMenu() {
  const [open, setOpen] = useState(false);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    api.getAutostart().then(setAutostart, (e) => setError(String(e)));
    const close = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  return (
    <div className="settings" ref={box}>
      <button type="button" className={`rail-btn${open ? " on" : ""}`} onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Gear size={20} strokeWidth={1.75} />
        <span>{t.settings}</span>
      </button>
      {open && (
        <div className="popover settings-pop" role="dialog" aria-label={t.settings}>
          <Toggle
            label={t.autostart}
            tip={t.autostartTip}
            checked={!!autostart}
            onChange={(v) => {
              setError(null);
              api.setAutostart(v).then(setAutostart, (e) => setError(String(e)));
            }}
          />
          <p className="muted small">{t.autostartTip}</p>
          {error && <p className="error small">{error}</p>}
        </div>
      )}
    </div>
  );
}
