import type { ReactNode } from "react";
import type { Content } from "../api";

export interface ToolCtx {
  width: number;
  dpi: number;
  labelCapable: boolean;
}

export interface Output {
  pages: HTMLCanvasElement[];
  content: Content;
  label?: boolean;
  labelHeightMm?: number;
}

export interface ToolProps {
  ctx: ToolCtx;
  onOutput: (o: Output) => void;
}

export function Field({ label, children, tip }: { label: string; children: ReactNode; tip?: string }) {
  return (
    <label className="field" title={tip}>
      <span className="field-label">{label}</span>
      {children}
    </label>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: ReactNode; tip?: string }[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          className={value === o.value ? "on" : ""}
          title={o.tip}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Toggle({ checked, onChange, label, tip }: { checked: boolean; onChange: (v: boolean) => void; label: string; tip?: string }) {
  return (
    <label className="toggle" title={tip}>
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span className="toggle-track" aria-hidden />
      <span>{label}</span>
    </label>
  );
}

export function Range({
  value,
  min,
  max,
  step = 1,
  onChange,
  label,
  tip,
  format,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (v: number) => void;
  label: string;
  tip?: string;
  format?: (v: number) => string;
}) {
  return (
    <label className="range" title={tip}>
      <span className="field-label">
        {label}
        <output>{format ? format(value) : value}</output>
      </span>
      <input type="range" min={min} max={max} step={step} value={value} onChange={(e) => onChange(Number(e.target.value))} />
    </label>
  );
}

export function IconButton({ onClick, label, children, disabled }: { onClick: () => void; label: string; children: ReactNode; disabled?: boolean }) {
  return (
    <button type="button" className="icon-btn" onClick={onClick} aria-label={label} title={label} disabled={disabled}>
      {children}
    </button>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="empty">{children}</div>;
}
