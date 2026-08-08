/* embyr console — Billing (usage only, no pricing) */
const BillingView = (function () {
  const { useState } = React;
  const Icon = window.Icon;

  const RANGE_MULT = { "7d": 0.23, "30d": 1, "month": 0.78, "lastmonth": 1.02 };

  function UsageBar({ reads, writes, deletes, max }) {
    const total = reads + writes + deletes;
    const w = (v) => `${(v / max) * 100}%`;
    return (
      <div style={{ display: "flex", height: 7, borderRadius: 99, overflow: "hidden", background: "var(--surface-2)", width: 140 }} title={`${total.toLocaleString()} ops`}>
        <div style={{ width: w(reads), background: "var(--blue)" }} />
        <div style={{ width: w(writes), background: "var(--accent)" }} />
        <div style={{ width: w(deletes), background: "var(--red)" }} />
      </div>
    );
  }

  function Billing() {
    const { databases, fmtNum, account } = window.useApp();
    const [range, setRange] = useState("30d");
    const m = RANGE_MULT[range];

    const rows = databases.map((d) => ({
      name: d.name, status: d.status,
      reads: Math.round(d.reads * 30 * m), writes: Math.round(d.writes * 30 * m),
      deletes: Math.round(d.deletes * 30 * m), peakConn: d.activeStreams,
      logStorage: +(d.logStorageGB).toFixed(1),
    }));
    const totals = rows.reduce((t, r) => ({
      reads: t.reads + r.reads, writes: t.writes + r.writes, deletes: t.deletes + r.deletes,
      logStorage: t.logStorage + r.logStorage,
    }), { reads: 0, writes: 0, deletes: 0, logStorage: 0 });
    const maxOps = Math.max(...rows.map((r) => r.reads + r.writes + r.deletes), 1);

    return (
      <div className="page page-wide">
        <div className="page-head">
          <div>
            <h1 className="page-title">Billing</h1>
            <p className="page-sub">Raw usage quantities for <strong style={{ color: "var(--text)", fontWeight: 550 }}>{account.name}</strong>. Rates and invoices are handled separately.</p>
          </div>
          <div className="page-head-actions">
            <window.Select value={range} onChange={(e) => setRange(e.target.value)} style={{ width: 150 }}>
              <option value="7d">Last 7 days</option><option value="30d">Last 30 days</option>
              <option value="month">This month</option><option value="lastmonth">Last month</option>
            </window.Select>
          </div>
        </div>

        <div className="grid" style={{ gridTemplateColumns: "repeat(5, 1fr)", marginBottom: 24 }}>
          {[
            { icon: "zap", label: "Read ops", v: fmtNum(totals.reads), tone: "blue" },
            { icon: "activity", label: "Write ops", v: fmtNum(totals.writes), tone: "accent" },
            { icon: "trash", label: "Delete ops", v: fmtNum(totals.deletes), tone: "red" },
            { icon: "link", label: "Peak connections", v: <span style={{ color: "var(--text-3)" }}>&mdash;</span>, foot: "V2" },
            { icon: "database", label: "Log storage", v: totals.logStorage.toFixed(1) + " GB" },
          ].map((s) => (
            <div className="kpi" key={s.label}>
              <div className="kpi-label"><Icon name={s.icon} size={14} />{s.label}{s.foot && <span style={{ marginLeft: "auto", fontSize: 9.5, fontWeight: 600, letterSpacing: ".04em", color: "var(--text-3)", background: "var(--surface-2)", padding: "1px 5px", borderRadius: 4 }}>{s.foot}</span>}</div>
              <div className="kpi-value" style={{ fontSize: 23 }}>{s.v}</div>
            </div>
          ))}
        </div>

        <div className="tbl-wrap">
          <table className="tbl">
            <thead><tr>
              <th>Database</th><th style={{ width: 160 }}>Mix</th>
              <th className="td-right">Read ops</th><th className="td-right">Write ops</th><th className="td-right">Delete ops</th>
              <th className="td-right">Peak conn.</th><th className="td-right">Log storage</th>
            </tr></thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.name}>
                  <td><span className="row" style={{ gap: 9 }}><Icon name="database" size={14} style={{ color: "var(--text-3)" }} /><span className="mono" style={{ fontWeight: 550, fontSize: 12.5, whiteSpace: "nowrap" }}>{r.name}</span></span></td>
                  <td><UsageBar reads={r.reads} writes={r.writes} deletes={r.deletes} max={maxOps} /></td>
                  <td className="td-right tnum">{fmtNum(r.reads)}</td>
                  <td className="td-right tnum">{fmtNum(r.writes)}</td>
                  <td className="td-right tnum">{fmtNum(r.deletes)}</td>
                  <td className="td-right tnum">{r.status === "active" ? r.peakConn : "—"}</td>
                  <td className="td-right tnum">{r.logStorage > 0 ? r.logStorage.toFixed(1) + " GB" : "—"}</td>
                </tr>
              ))}
            </tbody>
            <tfoot>
              <tr style={{ background: "var(--bg-elev)" }}>
                <td style={{ fontWeight: 650 }}>Total</td><td></td>
                <td className="td-right tnum" style={{ fontWeight: 650 }}>{fmtNum(totals.reads)}</td>
                <td className="td-right tnum" style={{ fontWeight: 650 }}>{fmtNum(totals.writes)}</td>
                <td className="td-right tnum" style={{ fontWeight: 650 }}>{fmtNum(totals.deletes)}</td>
                <td className="td-right tnum faint" style={{ fontWeight: 650 }}>—</td>
                <td className="td-right tnum" style={{ fontWeight: 650 }}>{totals.logStorage.toFixed(1)} GB</td>
              </tr>
            </tfoot>
          </table>
        </div>

        <div className="row" style={{ marginTop: 16, gap: 18, fontSize: 12, color: "var(--text-3)" }}>
          <span className="row" style={{ gap: 6 }}><span style={{ width: 9, height: 9, borderRadius: 3, background: "var(--blue)" }} />Reads</span>
          <span className="row" style={{ gap: 6 }}><span style={{ width: 9, height: 9, borderRadius: 3, background: "var(--accent)" }} />Writes</span>
          <span className="row" style={{ gap: 6 }}><span style={{ width: 9, height: 9, borderRadius: 3, background: "var(--red)" }} />Deletes</span>
          <div className="spacer" />
          <span className="row" style={{ gap: 6 }}><Icon name="alert" size={13} />Source: daily_project_metrics · log_storage_bytes column · peak connections is a daily snapshot</span>
        </div>
      </div>
    );
  }
  return Billing;
})();
window.BillingView = BillingView;
