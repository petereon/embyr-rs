/* embyr console — Query Logs */
const DbLogs = (function () {
  const { useState, useMemo } = React;
  const Icon = window.Icon;

  const OP_TONE = { read: "blue", write: "accent", delete: "red", query: "neutral", listen: "amber", subscribe: "green" };

  function EnablePanel({ db }) {
    const { patchDb } = window.useApp();
    const toast = window.useToast();
    const [retention, setRetention] = useState(7);
    return (
      <div className="card" style={{ maxWidth: 560, margin: "12px auto" }}>
        <div className="card-pad" style={{ textAlign: "center", padding: "36px 32px" }}>
          <div style={{ width: 48, height: 48, borderRadius: 12, background: "var(--accent-soft)", color: "var(--accent)", display: "grid", placeItems: "center", margin: "0 auto 16px" }}><Icon name="list" size={22} /></div>
          <h3 style={{ margin: "0 0 6px", fontSize: 16 }}>Query logging is off</h3>
          <p style={{ margin: "0 auto 20px", color: "var(--text-2)", fontSize: 13, maxWidth: 380 }}>
            Enable to record every operation against <b className="mono">{db.name}</b>. Log entries are billed to your account as storage at the document-storage rate.
          </p>
          <div className="field" style={{ maxWidth: 280, margin: "0 auto 20px", textAlign: "left" }}>
            <label className="field-label">Retention</label>
            <window.Segmented value={retention} onChange={setRetention} options={[{ value: 1, label: "1 day" }, { value: 7, label: "7 days" }, { value: 30, label: "30 days" }]} />
          </div>
          <window.Button variant="primary" icon="list" onClick={() => { patchDb(db.id, { logging: true, retention, logStorageGB: db.logStorageGB || 0.1 }); toast({ msg: "Query logging enabled", icon: "list" }); }}>Enable logging</window.Button>
        </div>
      </div>
    );
  }

  function LogTable({ db }) {
    const { patchDb } = window.useApp();
    const toast = window.useToast();
    const all = window.DATA.queryLogs[db.id] || window.DATA.queryLogs.db_prod_api;
    const [op, setOp] = useState("all");
    const [status, setStatus] = useState("all");
    const [pathQ, setPathQ] = useState("");
    const [sort, setSort] = useState({ key: "ts", dir: "desc" });

    const rows = useMemo(() => {
      let r = all.filter((l) =>
        (op === "all" || l.op === op) &&
        (status === "all" || (status === "ok" ? l.status === "ok" : l.status !== "ok")) &&
        (!pathQ || l.path.startsWith(pathQ)));
      r = [...r].sort((a, b) => {
        const m = sort.dir === "asc" ? 1 : -1;
        return (a[sort.key] - b[sort.key]) * m;
      });
      return r;
    }, [all, op, status, pathQ, sort]);

    const shown = rows.slice(0, 200);
    function toggleSort(key) { setSort((s) => s.key === key ? { key, dir: s.dir === "asc" ? "desc" : "asc" } : { key, dir: "desc" }); }
    function sortIcon(key) { return sort.key === key ? <Icon name={sort.dir === "asc" ? "chevronDown" : "chevronDown"} size={12} style={{ transform: sort.dir === "asc" ? "rotate(180deg)" : "none", color: "var(--accent)" }} /> : null; }

    return (
      <div className="fade-in">
        <div className="row" style={{ marginBottom: 14, flexWrap: "wrap", gap: 10 }}>
          <div style={{ width: 220 }}><window.Input icon="filter" placeholder="Collection path prefix…" value={pathQ} onChange={(e) => setPathQ(e.target.value)} /></div>
          <window.Select value={op} onChange={(e) => setOp(e.target.value)} style={{ width: 130 }}>
            <option value="all">All ops</option>
            {window.DATA.OPS.map((o) => <option key={o} value={o}>{o}</option>)}
          </window.Select>
          <window.Select value={status} onChange={(e) => setStatus(e.target.value)} style={{ width: 130 }}>
            <option value="all">Any status</option><option value="ok">ok</option><option value="err">errors</option>
          </window.Select>
          <div className="spacer" />
          <span style={{ fontSize: 12, color: "var(--text-3)" }} className="tnum">{rows.length.toLocaleString()} entries</span>
          <window.Button size="sm" variant="ghost" icon="download" onClick={() => toast({ msg: `Exporting ${Math.min(rows.length, 10000).toLocaleString()} rows as CSV`, icon: "download" })}>Export CSV</window.Button>
          <window.Menu trigger={<button className="btn btn-sm btn-ghost"><Icon name="settings" size={14} />Logging</button>} width={220}>
            <div className="menu-label">Retention · {db.retention} days</div>
            <window.MenuItem icon="pause" danger onClick={() => { patchDb(db.id, { logging: false }); toast({ msg: "Query logging disabled", icon: "list", tone: "amber" }); }}>Disable logging</window.MenuItem>
          </window.Menu>
        </div>

        <div className="tbl-wrap">
          <table className="tbl">
            <thead><tr>
              <th className="sortable" style={{ width: 150 }} onClick={() => toggleSort("ts")}><span className="row" style={{ gap: 4 }}>Timestamp {sortIcon("ts")}</span></th>
              <th style={{ width: 110 }}>Operation</th>
              <th>Collection path</th>
              <th className="sortable td-right" style={{ width: 110 }} onClick={() => toggleSort("duration")}><span className="row" style={{ gap: 4, justifyContent: "flex-end" }}>Duration {sortIcon("duration")}</span></th>
              <th style={{ width: 130 }}>Status</th>
              <th style={{ width: 90 }}>Client</th>
            </tr></thead>
            <tbody>
              {shown.map((l) => (
                <tr key={l.id}>
                  <td className="mono td-dim" style={{ fontSize: 11.5 }}>{window.fmtClock(l.ts)}</td>
                  <td><span className={`badge badge-${OP_TONE[l.op]}`} style={{ fontFamily: "var(--font-mono)", fontSize: 11 }}>{l.op}</span></td>
                  <td className="mono" style={{ fontSize: 12 }}>{l.path}</td>
                  <td className="td-right tnum" style={{ color: l.duration > 120 ? "var(--amber)" : "var(--text)" }}>{l.duration} ms</td>
                  <td>{l.status === "ok"
                    ? <span className="row" style={{ gap: 6, color: "var(--green)", fontSize: 12.5 }}><Icon name="circleCheck" size={13} />ok</span>
                    : <span className="badge badge-red" style={{ fontFamily: "var(--font-mono)", fontSize: 10.5 }}>{l.status}</span>}</td>
                  <td className="mono td-dim" style={{ fontSize: 11.5 }}>…{l.client}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length > 200 && <div style={{ padding: "12px 16px", textAlign: "center", fontSize: 12, color: "var(--text-3)", borderTop: "1px solid var(--border)" }}>Showing first 200 of {rows.length.toLocaleString()} · table virtualizes in production · export up to 10,000</div>}
          {rows.length === 0 && <div className="empty"><div className="empty-ico"><Icon name="search" size={20} /></div>No log entries match these filters.</div>}
        </div>
      </div>
    );
  }

  function QueryLogs({ db }) {
    return db.logging ? <LogTable db={db} /> : <EnablePanel db={db} />;
  }
  return QueryLogs;
})();
window.DbLogs = DbLogs;
