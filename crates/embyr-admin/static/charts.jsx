/* embyr console — charts (dependency-free SVG). Exports to window. */
const { useState: useStateC, useRef: useRefC, useMemo } = React;

/* path builders */
function linePath(data, w, h, pad = 0) {
  const max = Math.max(...data, 1), min = Math.min(...data, 0);
  const rng = max - min || 1;
  const n = data.length;
  return data.map((v, i) => {
    const x = (i / (n - 1)) * w;
    const y = pad + (h - pad * 2) * (1 - (v - min) / rng);
    return `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(" ");
}

/* ---------- Sparkline ---------- */
function Sparkline({ data, w = 120, h = 30, color = "var(--accent)", fill = true, strokeW = 1.5 }) {
  const id = useMemo(() => "sp" + Math.random().toString(36).slice(2), []);
  if (!data || data.every((d) => d === 0)) {
    return <svg width={w} height={h}><line x1="0" y1={h - 1} x2={w} y2={h - 1} stroke="var(--border-2)" strokeDasharray="3 3" /></svg>;
  }
  const lp = linePath(data, w, h, 2);
  const area = `${lp} L${w},${h} L0,${h} Z`;
  return (
    <svg width={w} height={h} style={{ display: "block", overflow: "visible" }}>
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.22" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      {fill && <path d={area} fill={`url(#${id})`} />}
      <path d={lp} fill="none" stroke={color} strokeWidth={strokeW} strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}

/* ---------- LatencyChart (hero) — area + hover crosshair ---------- */
function LatencyChart({ data, height = 200, unit = "ms", color = "var(--accent)", labels }) {
  const wrapRef = useRefC(null);
  const [hover, setHover] = useState(null);
  const W = 1000, H = height; // viewBox units, scales to width
  const id = useMemo(() => "lc" + Math.random().toString(36).slice(2), []);
  const max = Math.max(...data, 1), min = 0;
  const rng = max - min || 1;
  const n = data.length;
  const padTop = 16, padBot = 22;
  const xAt = (i) => (i / (n - 1)) * W;
  const yAt = (v) => padTop + (H - padTop - padBot) * (1 - (v - min) / rng);
  const lp = data.map((v, i) => `${i === 0 ? "M" : "L"}${xAt(i).toFixed(1)},${yAt(v).toFixed(1)}`).join(" ");
  const area = `${lp} L${W},${H - padBot} L0,${H - padBot} Z`;

  // gridlines
  const ticks = 4;
  const gridVals = Array.from({ length: ticks + 1 }, (_, i) => Math.round((max / ticks) * i));

  function onMove(e) {
    const rect = wrapRef.current.getBoundingClientRect();
    const px = (e.clientX - rect.left) / rect.width;
    const i = Math.max(0, Math.min(n - 1, Math.round(px * (n - 1))));
    setHover(i);
  }
  return (
    <div ref={wrapRef} style={{ position: "relative", width: "100%" }}
      onMouseMove={onMove} onMouseLeave={() => setHover(null)}>
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={height} preserveAspectRatio="none" style={{ display: "block", overflow: "visible" }}>
        <defs>
          <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity="0.26" />
            <stop offset="100%" stopColor={color} stopOpacity="0" />
          </linearGradient>
        </defs>
        {gridVals.map((gv, i) => {
          const y = yAt(gv);
          return <line key={i} x1="0" y1={y} x2={W} y2={y} stroke="var(--border)" strokeWidth="1" vectorEffect="non-scaling-stroke" />;
        })}
        <path d={area} fill={`url(#${id})`} />
        <path d={lp} fill="none" stroke={color} strokeWidth="2" vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap="round" />
        {hover != null && (
          <g>
            <line x1={xAt(hover)} y1={padTop - 6} x2={xAt(hover)} y2={H - padBot} stroke="var(--text-3)" strokeWidth="1" vectorEffect="non-scaling-stroke" strokeDasharray="3 3" />
            <circle cx={xAt(hover)} cy={yAt(data[hover])} r="3.5" fill={color} stroke="var(--bg-elev)" strokeWidth="2" vectorEffect="non-scaling-stroke" />
          </g>
        )}
      </svg>
      {/* y-axis labels overlay */}
      <div style={{ position: "absolute", inset: 0, pointerEvents: "none" }}>
        {gridVals.map((gv, i) => (
          <span key={i} className="mono tnum" style={{ position: "absolute", left: 0, top: `${(yAt(gv) / H) * 100}%`,
            transform: "translateY(-50%)", fontSize: 10.5, color: "var(--text-3)", background: "var(--surface)", padding: "0 4px" }}>{gv}</span>
        ))}
      </div>
      {/* x labels */}
      {labels && (
        <div className="row" style={{ justifyContent: "space-between", marginTop: 6, fontSize: 10.5, color: "var(--text-3)" }}>
          {labels.map((l, i) => <span key={i} className="mono">{l}</span>)}
        </div>
      )}
      {/* tooltip */}
      {hover != null && (
        <div style={{
          position: "absolute", top: 0, left: `${(xAt(hover) / W) * 100}%`,
          marginLeft: hover > n / 2 ? -110 : 10, pointerEvents: "none",
          background: "var(--bg-elev)", border: "1px solid var(--border-2)", borderRadius: 8, padding: "6px 10px",
          boxShadow: "var(--shadow-2)", whiteSpace: "nowrap", zIndex: 5,
        }}>
          <div style={{ fontSize: 16, fontWeight: 600 }} className="tnum">{data[hover]}<span style={{ fontSize: 11, color: "var(--text-3)", marginLeft: 2 }}>{unit}</span></div>
          <div style={{ fontSize: 10.5, color: "var(--text-3)" }} className="mono">{n - 1 - hover}m ago</div>
        </div>
      )}
    </div>
  );
}

/* ---------- BarChart (ops by hour) ---------- */
function BarChart({ data, height = 160, color = "var(--accent)", labels }) {
  const max = Math.max(...data, 1);
  return (
    <div style={{ width: "100%" }}>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 3, height }}>
        {data.map((v, i) => (
          <div key={i} title={`${v.toLocaleString()} ops`} style={{
            flex: 1, height: `${Math.max(2, (v / max) * 100)}%`, borderRadius: "3px 3px 1px 1px",
            background: i === data.length - 1 ? color : `color-mix(in oklab, ${color} 55%, var(--surface-3))`,
            transition: "height .3s, background .15s", cursor: "default",
          }} />
        ))}
      </div>
      {labels && (
        <div className="row" style={{ justifyContent: "space-between", marginTop: 8, fontSize: 10.5, color: "var(--text-3)" }}>
          {labels.map((l, i) => <span key={i} className="mono">{l}</span>)}
        </div>
      )}
    </div>
  );
}

/* ---------- Donut (connection breakdown) ---------- */
function Donut({ segments, size = 130, thickness = 16, centerLabel, centerSub }) {
  const total = segments.reduce((s, x) => s + x.value, 0) || 1;
  const r = (size - thickness) / 2;
  const c = 2 * Math.PI * r;
  let offset = 0;
  return (
    <div style={{ position: "relative", width: size, height: size, flexShrink: 0 }}>
      <svg width={size} height={size} style={{ transform: "rotate(-90deg)" }}>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--surface-3)" strokeWidth={thickness} />
        {segments.map((s, i) => {
          const len = (s.value / total) * c;
          const el = <circle key={i} cx={size / 2} cy={size / 2} r={r} fill="none" stroke={s.color}
            strokeWidth={thickness} strokeDasharray={`${len} ${c - len}`} strokeDashoffset={-offset} strokeLinecap="round" />;
          offset += len;
          return el;
        })}
      </svg>
      <div style={{ position: "absolute", inset: 0, display: "grid", placeItems: "center", textAlign: "center" }}>
        <div>
          <div style={{ fontSize: 24, fontWeight: 600, lineHeight: 1 }} className="tnum">{centerLabel}</div>
          {centerSub && <div style={{ fontSize: 11, color: "var(--text-3)", marginTop: 3 }}>{centerSub}</div>}
        </div>
      </div>
    </div>
  );
}

window.Sparkline = Sparkline;
window.LatencyChart = LatencyChart;
window.BarChart = BarChart;
window.Donut = Donut;
