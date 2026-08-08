/* embyr console — Dashboard view */
const DashView = (function () {
  const { useState } = React;
  const Icon = window.Icon;

  function SummaryStat({ icon, label, value, unit, tone, foot }) {
    return (
      <div className="kpi">
        <div className="kpi-label"><Icon name={icon} size={14} />{label}{foot && <span style={{ marginLeft: "auto", fontSize: 9.5, fontWeight: 600, letterSpacing: ".04em", color: "var(--text-3)", background: "var(--surface-2)", padding: "1px 5px", borderRadius: 4 }}>{foot}</span>}</div>
        <div className="kpi-value" style={tone ? { color: `var(--${tone})` } : null}>
          {value}{unit && <span className="unit">{unit}</span>}
        </div>
      </div>
    );
  }

  function DbCard({ db }) {
    const { openDb, fmtNum } = window.useApp();
    const dead = db.status !== "active";
    return (
      <button className="card fade-in" onClick={() => openDb(db.id)} style={{
        textAlign: "left", cursor: "pointer", padding: 0, overflow: "hidden",
        display: "flex", flexDirection: "column", transition: "border-color .14s, transform .06s",
      }} onMouseEnter={(e) => e.currentTarget.style.borderColor = "var(--border-2)"}
        onMouseLeave={(e) => e.currentTarget.style.borderColor = "var(--border)"}>
        <div style={{ padding: "16px 18px 12px", display: "flex", alignItems: "center", gap: 10 }}>
          <span style={{ width: 30, height: 30, borderRadius: 8, background: "var(--accent-soft)", color: "var(--accent)", display: "grid", placeItems: "center", flexShrink: 0 }}>
            <Icon name="database" size={15} />
          </span>
          <div style={{ minWidth: 0, flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 14.5, letterSpacing: "-0.01em", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{db.name}</div>
            <div className="mono" style={{ fontSize: 11, color: "var(--text-3)" }}>{db.region}</div>
          </div>
          <window.StatusBadge status={db.status} />
        </div>

        <div style={{ padding: "0 18px 4px" }}>
          <window.Sparkline data={db.latency} w={300} h={42} color={dead ? "var(--text-3)" : "var(--accent)"} />
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 1, background: "var(--border)", borderTop: "1px solid var(--border)", marginTop: 8 }}>
          <div style={{ background: "var(--surface)", padding: "11px 18px" }}>
            <div style={{ fontSize: 11, color: "var(--text-3)" }}>P95 read</div>
            <div className="tnum" style={{ fontSize: 18, fontWeight: 600, marginTop: 2 }}>
              {dead ? "—" : db.p95}<span style={{ fontSize: 11, color: "var(--text-3)", marginLeft: 2, fontWeight: 500 }}>ms</span>
            </div>
          </div>
          <div style={{ background: "var(--surface)", padding: "11px 18px" }}>
            <div style={{ fontSize: 11, color: "var(--text-3)" }}>Live streams</div>
            <div className="tnum" style={{ fontSize: 18, fontWeight: 600, marginTop: 2, color: "var(--text-3)" }} title="Deferred to V2 — multi-pod counter">&mdash;<span style={{ fontSize: 9.5, color: "var(--text-3)", marginLeft: 6, fontWeight: 600, letterSpacing: ".04em", verticalAlign: "middle", background: "var(--surface-2)", padding: "1px 5px", borderRadius: 4 }}>V2</span></div>
          </div>
        </div>

        <div style={{ display: "flex", gap: 14, padding: "11px 18px", borderTop: "1px solid var(--border)", fontSize: 11.5, color: "var(--text-2)" }}>
          <span><span className="tnum" style={{ color: "var(--text)", fontWeight: 550 }}>{fmtNum(db.reads)}</span> reads</span>
          <span><span className="tnum" style={{ color: "var(--text)", fontWeight: 550 }}>{fmtNum(db.writes)}</span> writes</span>
          <span><span className="tnum" style={{ color: "var(--text)", fontWeight: 550 }}>{fmtNum(db.deletes)}</span> deletes</span>
        </div>
      </button>
    );
  }

  function Dashboard() {
    const { databases, go, fmtNum, account } = window.useApp();
    const [newOpen, setNewOpen] = useState(false);
    const active = databases.filter((d) => d.status === "active");
    const totalReads = databases.reduce((s, d) => s + d.reads, 0);
    const avgP95 = active.length ? Math.round(active.reduce((s, d) => s + d.p95, 0) / active.length) : 0;

    return (
      <div className="page page-wide">
        <div className="page-head">
          <div>
            <h1 className="page-title">Dashboard</h1>
            <p className="page-sub">Overview of all databases in <strong style={{ color: "var(--text)", fontWeight: 550 }}>{account.name}</strong> · live as of just now</p>
          </div>
          <div className="page-head-actions">
            <window.Button variant="ghost" icon="refresh" size="sm">Refresh</window.Button>
            <window.Button variant="primary" icon="plus" onClick={() => setNewOpen(true)}>New database</window.Button>
          </div>
        </div>

        <div className="grid" style={{ gridTemplateColumns: "repeat(4, 1fr)", marginBottom: 28 }}>
          <SummaryStat icon="database" label="Databases" value={databases.length} />
          <SummaryStat icon="activity" label="Active streams" value={<span style={{ color: "var(--text-3)" }}>&mdash;</span>} foot="V2" />
          <SummaryStat icon="zap" label="Reads today" value={fmtNum(totalReads)} />
          <SummaryStat icon="gauge" label="Avg P95 read" value={avgP95} unit="ms" tone={avgP95 > 25 ? "amber" : "green"} />
        </div>

        <div className="row" style={{ marginBottom: 14 }}>
          <h2 style={{ fontSize: 14, fontWeight: 600, margin: 0 }}>Databases</h2>
          <span className="tag" style={{ marginLeft: 2 }}>{databases.length}</span>
          <div className="spacer" />
          <button className="btn btn-ghost btn-sm" onClick={() => go("databases")}>View all<Icon name="chevronRight" size={14} /></button>
        </div>

        <div className="grid" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(300px, 1fr))" }}>
          {databases.map((db) => <DbCard key={db.id} db={db} />)}
          <button className="card" onClick={() => setNewOpen(true)} style={{
            display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 10,
            minHeight: 180, cursor: "pointer", color: "var(--text-3)", borderStyle: "dashed", background: "transparent",
          }} onMouseEnter={(e) => { e.currentTarget.style.color = "var(--accent)"; e.currentTarget.style.borderColor = "var(--accent-line)"; }}
            onMouseLeave={(e) => { e.currentTarget.style.color = "var(--text-3)"; e.currentTarget.style.borderColor = "var(--border)"; }}>
            <Icon name="plus" size={22} />
            <span style={{ fontSize: 13, fontWeight: 550 }}>Create database</span>
          </button>
        </div>

        {newOpen && <window.CreateDatabaseModal onClose={() => setNewOpen(false)} />}
      </div>
    );
  }

  return Dashboard;
})();
window.DashView = DashView;
