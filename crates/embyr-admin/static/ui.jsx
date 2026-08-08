/* embyr console — UI primitives. Exports to window. */
const { useState, useEffect, useRef, useCallback, createContext, useContext } = React;
const Icon = window.Icon;

/* ---------- formatting helpers ---------- */
function relTime(iso) {
  if (!iso) return "—";
  const d = typeof iso === "number" ? iso : Date.parse(iso);
  const s = Math.floor((Date.now() - d) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return Math.floor(s / 60) + "m ago";
  if (s < 86400) return Math.floor(s / 3600) + "h ago";
  if (s < 2592000) return Math.floor(s / 86400) + "d ago";
  return new Date(d).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}
function fmtDate(iso) {
  if (!iso) return "—";
  return new Date(iso).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
}
function fmtClock(ts) {
  const d = new Date(ts);
  return d.toLocaleTimeString("en-GB", { hour12: false }) + "." + String(d.getMilliseconds()).padStart(3, "0");
}

/* ---------- Button ---------- */
function Button({ variant = "default", size, icon, iconRight, children, block, className = "", ...rest }) {
  const cls = ["btn",
    variant === "primary" && "btn-primary",
    variant === "ghost" && "btn-ghost",
    variant === "danger" && "btn-danger",
    size === "sm" && "btn-sm", size === "lg" && "btn-lg",
    block && "btn-block",
    !children && "btn-icon-only",
    className].filter(Boolean).join(" ");
  return (
    <button className={cls} {...rest}>
      {icon && <Icon name={icon} size={size === "sm" ? 14 : 15} />}
      {children}
      {iconRight && <Icon name={iconRight} size={size === "sm" ? 14 : 15} />}
    </button>
  );
}

/* ---------- Badge / StatusBadge / RoleBadge ---------- */
function Badge({ tone = "neutral", dot, children, className = "" }) {
  return <span className={`badge badge-${tone} ${className}`}>{dot && <span className="dot" />}{children}</span>;
}
const STATUS_TONE = { active: "green", suspended: "amber", deleted: "red", pending: "blue", ok: "green", error: "red" };
function StatusBadge({ status }) {
  const tone = STATUS_TONE[status] || "neutral";
  return <Badge tone={tone} dot>{status}</Badge>;
}
const ROLE_TONE = { owner: "accent", admin: "blue", viewer: "neutral" };
function RoleBadge({ role }) {
  return <Badge tone={ROLE_TONE[role] || "neutral"} className="badge-role">{role}</Badge>;
}

/* ---------- Avatar ---------- */
function Avatar({ name, size = 28, square }) {
  const initial = (name || "?").trim()[0]?.toUpperCase() || "?";
  return (
    <span style={{
      width: size, height: size, borderRadius: square ? "7px" : "50%",
      display: "grid", placeItems: "center", flexShrink: 0,
      background: "var(--surface-3)", color: "var(--text-2)", fontWeight: 600,
      fontSize: size * 0.42, border: "1px solid var(--border)",
    }}>{initial}</span>
  );
}

/* ---------- Card ---------- */
function Card({ title, sub, actions, pad, children, className = "", style }) {
  return (
    <div className={`card ${className}`} style={style}>
      {(title || actions) && (
        <div className="card-head">
          <div className="col" style={{ gap: 2 }}>
            {title && <h3>{title}</h3>}
            {sub && <span className="sub">{sub}</span>}
          </div>
          {actions && <div className="row" style={{ marginLeft: "auto", gap: 8 }}>{actions}</div>}
        </div>
      )}
      <div className={pad ? "card-pad" : ""}>{children}</div>
    </div>
  );
}

/* ---------- Toggle ---------- */
function Toggle({ on, onChange, disabled }) {
  return <button className={`toggle ${on ? "on" : ""}`} disabled={disabled}
    onClick={() => !disabled && onChange(!on)} aria-pressed={on} />;
}

/* ---------- Segmented ---------- */
function Segmented({ value, onChange, options }) {
  return (
    <div className="seg">
      {options.map((o) => (
        <button key={o.value} className={value === o.value ? "on" : ""} onClick={() => onChange(o.value)}>
          {o.icon && <Icon name={o.icon} size={14} />}{o.label}
        </button>
      ))}
    </div>
  );
}

/* ---------- Tabs ---------- */
function Tabs({ value, onChange, tabs }) {
  return (
    <div className="tabs">
      {tabs.map((t) => (
        <button key={t.value} className={`tab ${value === t.value ? "active" : ""}`} onClick={() => onChange(t.value)}>
          {t.icon && <Icon name={t.icon} size={15} />}{t.label}
          {t.count != null && <span className="tab-count tnum">{t.count}</span>}
        </button>
      ))}
    </div>
  );
}

/* ---------- Field / Input / Select / Textarea ---------- */
function Field({ label, hint, error, children, htmlFor }) {
  return (
    <div className="field">
      {label && <label className="field-label" htmlFor={htmlFor}>{label}</label>}
      {children}
      {error ? <span className="field-err"><Icon name="alert" size={13} />{error}</span>
        : hint ? <span className="field-hint">{hint}</span> : null}
    </div>
  );
}
function Input({ error, mono, icon, className = "", ...rest }) {
  const el = <input className={`input ${mono ? "mono" : ""} ${error ? "input-err" : ""} ${className}`} {...rest} />;
  if (icon) return <div className="input-group"><Icon name={icon} size={15} />{el}</div>;
  return el;
}
function Select({ error, children, className = "", ...rest }) {
  return <select className={`select ${error ? "input-err" : ""} ${className}`} {...rest}>{children}</select>;
}

/* ---------- Dropdown menu ---------- */
function Menu({ trigger, children, align = "right", width }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);
  useEffect(() => {
    if (!open) return;
    const h = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener("mousedown", h);
    return () => document.removeEventListener("mousedown", h);
  }, [open]);
  return (
    <div ref={ref} style={{ position: "relative", display: "inline-flex" }}>
      {React.cloneElement(trigger, { onClick: (e) => { e.stopPropagation(); setOpen((o) => !o); } })}
      {open && (
        <div className="menu fade-in" style={{
          position: "absolute", top: "calc(100% + 6px)", [align]: 0, zIndex: 50, width,
          animationDuration: ".12s",
        }} onClick={() => setOpen(false)}>{children}</div>
      )}
    </div>
  );
}
function MenuItem({ icon, danger, children, ...rest }) {
  return <button className={`menu-item ${danger ? "danger" : ""}`} {...rest}>{icon && <Icon name={icon} size={15} />}{children}</button>;
}

/* ---------- Modal ---------- */
function Modal({ title, desc, icon, onClose, children, footer, size }) {
  useEffect(() => {
    const h = (e) => e.key === "Escape" && onClose?.();
    document.addEventListener("keydown", h);
    return () => document.removeEventListener("keydown", h);
  }, [onClose]);
  return (
    <div className="scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose?.()}>
      <div className={`modal ${size === "lg" ? "modal-lg" : ""}`}>
        {(title || icon) && (
          <div className="modal-head">
            {icon && <div style={{ width: 34, height: 34, borderRadius: 9, background: "var(--accent-soft)", color: "var(--accent)", display: "grid", placeItems: "center", flexShrink: 0 }}><Icon name={icon} size={17} /></div>}
            <div className="col" style={{ gap: 0, flex: 1 }}>
              <h2>{title}</h2>
              {desc && <p>{desc}</p>}
            </div>
            <button className="iconbtn" onClick={onClose}><Icon name="x" size={17} /></button>
          </div>
        )}
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-foot">{footer}</div>}
      </div>
    </div>
  );
}

/* ---------- CopyField ---------- */
function CopyField({ value, label }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    navigator.clipboard?.writeText(value).catch(() => {});
    setCopied(true); setTimeout(() => setCopied(false), 1400);
  };
  return (
    <div className="copyfield">
      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{value}</span>
      <button className="iconbtn ck" style={{ width: 28, height: 28 }} onClick={copy} title="Copy">
        <Icon name={copied ? "check" : "copy"} size={14} style={copied ? { color: "var(--green)" } : null} />
      </button>
    </div>
  );
}
function CopyInline({ value, children }) {
  const [copied, setCopied] = useState(false);
  return (
    <button className="row" style={{ background: "none", border: "none", cursor: "pointer", gap: 6, padding: 0, color: "inherit", font: "inherit" }}
      onClick={() => { navigator.clipboard?.writeText(value).catch(() => {}); setCopied(true); setTimeout(() => setCopied(false), 1200); }} title="Copy">
      {children}
      <Icon name={copied ? "check" : "copy"} size={12.5} style={{ color: copied ? "var(--green)" : "var(--text-3)" }} />
    </button>
  );
}

/* ---------- Toasts ---------- */
const ToastCtx = createContext(null);
function ToastProvider({ children }) {
  const [toasts, setToasts] = useState([]);
  const push = useCallback((t) => {
    const id = Math.random().toString(36).slice(2);
    setToasts((ts) => [...ts, { id, ...t }]);
    setTimeout(() => setToasts((ts) => ts.filter((x) => x.id !== id)), t.duration || 3200);
  }, []);
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div style={{ position: "fixed", bottom: 22, left: "50%", transform: "translateX(-50%)", zIndex: 300, display: "flex", flexDirection: "column", gap: 8, alignItems: "center" }}>
        {toasts.map((t) => (
          <div key={t.id} className="fade-in" style={{
            display: "flex", alignItems: "center", gap: 10, background: "var(--bg-elev)",
            border: "1px solid var(--border-2)", borderRadius: 10, padding: "10px 15px",
            boxShadow: "var(--shadow-pop)", fontSize: 13.5, fontWeight: 500, minWidth: 240,
          }}>
            <Icon name={t.icon || "circleCheck"} size={16} style={{ color: `var(--${t.tone || "green"})` }} />
            <span>{t.msg}</span>
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}
function useToast() { return useContext(ToastCtx); }

/* ---------- RevealKeyModal (shown once on key creation) ---------- */
function RevealKeyModal({ name, value, kind, onClose }) {
  return (
    <Modal title={`${kind} key created`} icon="key" size="lg"
      desc="Copy this key now — for security it will never be shown again." onClose={onClose}
      footer={<Button variant="primary" onClick={onClose}>Done</Button>}>
      <Field label="Name"><div className="copyfield" style={{ background: "var(--surface-2)" }}>{name}</div></Field>
      <Field label="Secret key">
        <CopyField value={value} />
        <span className="field-hint" style={{ display: "flex", gap: 6, alignItems: "center", color: "var(--amber)" }}>
          <Icon name="alert" size={13} /> Store it in a secret manager. embyr keeps only a BLAKE3 hash.
        </span>
      </Field>
    </Modal>
  );
}

/* ---------- expose ---------- */
Object.assign(window, {
  Button, Badge, StatusBadge, RoleBadge, Avatar, Card, Toggle, Segmented, Tabs,
  Field, Input, Select, Menu, MenuItem, Modal, CopyField, CopyInline, RevealKeyModal,
  ToastProvider, useToast, relTime, fmtDate, fmtClock,
});
