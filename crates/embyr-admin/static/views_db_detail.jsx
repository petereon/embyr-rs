/* embyr console — Database detail: wrapper + Overview (hero) + Connections + SDK Keys */
const DatabaseDetail = (function () {
  const { useState, useEffect } = React;
  const Icon = window.Icon;

  /* ============ shared little bits ============ */
  function StatPair({ label, value, unit, tone }) {
    return (
      <div className="col" style={{ gap: 3 }}>
        <span style={{ fontSize: 11.5, color: "var(--text-3)" }}>{label}</span>
        <span className="tnum" style={{ fontSize: 19, fontWeight: 600, color: tone ? `var(--${tone})` : "var(--text)" }}>
          {value}{unit && <span style={{ fontSize: 11, color: "var(--text-3)", fontWeight: 500, marginLeft: 2 }}>{unit}</span>}
        </span>
      </div>
    );
  }

  /* ============ HERO: Overview ============ */
  function DbOverview({ db }) {
    const { fmtNum, patchDb, openDb } = window.useApp();
    const toast = window.useToast();
    const [range, setRange] = useState("1h");
    const dead = db.status !== "active";

    const opLabels = ["−24h", "−18h", "−12h", "−6h", "now"];
    const hourLabels = Array.from({ length: 24 }, (_, i) => i).filter((i) => i % 6 === 0).map((i) => `${i.toString().padStart(2, "0")}:00`);

    return (
      <div className="fade-in">
        {/* KPI ROW */}
        <div className="grid" style={{ gridTemplateColumns: "1.35fr 1fr 1fr 1fr", marginBottom: 18 }}>
          <div className="kpi" style={{ display: "flex", flexDirection: "column" }}>
            <div className="kpi-label"><Icon name="gauge" size={14} />P95 read latency<span style={{ marginLeft: "auto", fontSize: 10.5, color: "var(--text-3)" }}>last hour</span></div>
            <div className="row" style={{ alignItems: "flex-end", gap: 12, marginTop: 6 }}>
              <div className="kpi-value" style={{ color: dead ? "var(--text-3)" : db.p95 > 25 ? "var(--amber)" : "var(--text)" }}>{dead ? "—" : db.p95}<span className="unit">ms</span></div>
              <div style={{ flex: 1, marginBottom: 4, minWidth: 0, overflow: "hidden" }}><window.Sparkline data={db.latency} w={170} h={38} color={dead ? "var(--text-3)" : "var(--accent)"} /></div>
            </div>
            <div className="kpi-foot">
              <span>p50 <b className="tnum" style={{ color: "var(--text-2)" }}>{db.p50}ms</b></span>
              <span style={{ color: "var(--border-2)" }}>·</span>
              <span>p99 <b className="tnum" style={{ color: "var(--text-2)" }}>{db.p99}ms</b></span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label"><Icon name="zap" size={14} />Reads today</div>
            <div className="kpi-value">{fmtNum(db.reads)}</div>
            <div className="kpi-foot"><span className="trend-up"><Icon name="arrowUpRight" size={12} /> 8.2%</span> vs yesterday</div>
          </div>
          <div className="kpi">
            <div className="kpi-label"><Icon name="activity" size={14} />Writes today</div>
            <div className="kpi-value">{fmtNum(db.writes)}</div>
            <div className="kpi-foot"><span className="faint">{fmtNum(db.deletes)} deletes</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label"><Icon name="link" size={14} />Active connections<span style={{ marginLeft: "auto", fontSize: 9.5, fontWeight: 600, letterSpacing: ".04em", color: "var(--text-3)", background: "var(--surface-2)", padding: "1px 5px", borderRadius: 4 }}>V2</span></div>
            <div className="kpi-value" style={{ color: "var(--text-3)" }}>&mdash;</div>
            <div className="kpi-foot"><span className="faint">Multi-pod counter coming in V2</span></div>
          </div>
        </div>

        {/* LATENCY CHART */}
        <div className="card" style={{ marginBottom: 18 }}>
          <div className="card-head">
            <div className="col" style={{ gap: 2 }}>
              <h3>Read latency</h3>
              <span className="sub">Per-minute P95, request receipt → response sent</span>
            </div>
            <div className="row" style={{ marginLeft: "auto" }}>
              <window.Segmented value={range} onChange={setRange} options={[{ value: "1h", label: "1h" }, { value: "6h", label: "6h" }, { value: "24h", label: "24h" }]} />
            </div>
          </div>
          <div className="card-pad">
            {dead ? <div className="empty"><div className="empty-ico"><Icon name="pause" size={20} /></div>Database is suspended — no live metrics.</div>
              : <window.LatencyChart data={db.latency} height={210} labels={["60m", "45m", "30m", "15m", "now"]} />}
          </div>
        </div>

        {/* TWO COLUMN: ops + connections */}
        <div className="grid" style={{ gridTemplateColumns: "1.6fr 1fr", marginBottom: 18 }}>
          <div className="card">
            <div className="card-head"><h3>Operations</h3><span className="sub" style={{ marginLeft: "auto" }}>last 24 hours</span></div>
            <div className="card-pad">
              <div className="row" style={{ gap: 28, marginBottom: 18 }}>
                <StatPair label="Reads" value={fmtNum(db.reads)} />
                <StatPair label="Writes" value={fmtNum(db.writes)} />
                <StatPair label="Deletes" value={fmtNum(db.deletes)} />
              </div>
              {dead ? <div className="empty" style={{ padding: 20 }}>No activity</div>
                : <window.BarChart data={db.ops} height={140} labels={opLabels} />}
            </div>
          </div>
          <div className="card">
            <div className="card-head"><h3>Live connections</h3><span className="badge badge-neutral" style={{ marginLeft: "auto" }}>V2</span></div>
            <div className="card-pad" style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", textAlign: "center", gap: 12, minHeight: 196 }}>
              <span style={{ width: 46, height: 46, borderRadius: 12, background: "var(--surface-2)", color: "var(--text-3)", display: "grid", placeItems: "center" }}><Icon name="link" size={21} /></span>
              <div className="tnum" style={{ fontSize: 34, fontWeight: 600, color: "var(--text-3)", lineHeight: 1 }}>&mdash;</div>
              <p style={{ margin: 0, fontSize: 12.5, color: "var(--text-3)", maxWidth: 230, lineHeight: 1.5 }}>
                Live Subscribe &amp; Listen counts are deferred to V2 — a shared cross-pod counter is required to aggregate accurately.
              </p>
            </div>
          </div>
        </div>

        {/* LOGGING + QUICK LINKS */}
        <div className="card card-pad" style={{ display: "flex", alignItems: "center", gap: 16 }}>
          <span style={{ width: 36, height: 36, borderRadius: 9, background: db.logging ? "var(--accent-soft)" : "var(--surface-2)", color: db.logging ? "var(--accent)" : "var(--text-3)", display: "grid", placeItems: "center" }}><Icon name="list" size={17} /></span>
          <div className="col" style={{ gap: 2 }}>
            <span style={{ fontWeight: 550, fontSize: 13.5 }}>Query logging</span>
            <span style={{ fontSize: 12, color: "var(--text-3)" }}>{db.logging ? `Enabled · ${db.retention}-day retention · billed as storage` : "Off — operations are not being recorded"}</span>
          </div>
          <div className="spacer" />
          {db.logging && <window.Button variant="ghost" size="sm" icon="arrowUpRight" onClick={() => openDb(db.id, "logs")}>View logs</window.Button>}
          <window.Toggle on={db.logging} onChange={(v) => { patchDb(db.id, { logging: v, logStorageGB: v ? db.logStorageGB || 0.1 : db.logStorageGB }); toast({ msg: v ? "Query logging enabled" : "Query logging disabled", icon: "list", tone: v ? "green" : "amber" }); }} />
        </div>
      </div>
    );
  }

  /* ============ Connections ============ */
  function maskDsn(url) {
    if (!url) return "—";
    return url; // already host:port/db in our data (no creds shown)
  }
  function DbConnections({ db }) {
    const { patchDb } = window.useApp();
    const toast = window.useToast();
    const canEdit = true;
    const [edit, setEdit] = useState(false);
    const [draft, setDraft] = useState(db);
    useEffect(() => setDraft(db), [db.id]);

    const isDirect = db.backendMode === "direct_pg";
    function Row({ label, children }) {
      return (
        <div style={{ display: "grid", gridTemplateColumns: "180px 1fr", gap: 16, padding: "14px 0", borderBottom: "1px solid var(--border)", alignItems: "center" }}>
          <span style={{ fontSize: 13, color: "var(--text-2)", fontWeight: 500 }}>{label}</span>
          <div>{children}</div>
        </div>
      );
    }

    return (
      <div className="fade-in grid" style={{ gridTemplateColumns: "1.5fr 1fr", alignItems: "start" }}>
        <div className="card">
          <div className="card-head">
            <div className="col" style={{ gap: 2 }}><h3>Backend configuration</h3><span className="sub">How this database reaches Postgres</span></div>
            {!edit
              ? <window.Button size="sm" variant="ghost" icon="settings" style={{ marginLeft: "auto" }} onClick={() => setEdit(true)}>Edit</window.Button>
              : <div className="row" style={{ marginLeft: "auto" }}><window.Button size="sm" variant="ghost" onClick={() => { setDraft(db); setEdit(false); }}>Cancel</window.Button><window.Button size="sm" variant="primary" onClick={() => { patchDb(db.id, draft); setEdit(false); toast({ msg: "Connection config saved", icon: "check" }); }}>Save</window.Button></div>}
          </div>
          <div className="card-pad" style={{ paddingTop: 4 }}>
            <Row label="Mode">
              {edit ? <window.Segmented value={draft.backendMode} onChange={(v) => setDraft({ ...draft, backendMode: v })} options={[{ value: "direct_pg", label: "Direct" }, { value: "agent_mode", label: "Agent" }]} />
                : <span className="badge badge-neutral" style={{ fontFamily: "var(--font-mono)" }}>{db.backendMode}</span>}
            </Row>
            {(edit ? draft.backendMode === "direct_pg" : isDirect) ? (
              <Row label="Connection URL">
                {edit ? <window.Input mono value={draft.connectionUrl || ""} onChange={(e) => setDraft({ ...draft, connectionUrl: e.target.value })} />
                  : <div className="row" style={{ gap: 8 }}><span className="mono" style={{ fontSize: 12.5 }}>{db.connectionUrl}</span><span className="tag" style={{ height: 18 }}>credentials masked</span></div>}
              </Row>
            ) : (
              <>
                <Row label="Agent endpoint">
                  {edit ? <window.Input mono value={draft.agentEndpoint || ""} onChange={(e) => setDraft({ ...draft, agentEndpoint: e.target.value })} />
                    : <span className="mono" style={{ fontSize: 12.5 }}>{db.agentEndpoint}</span>}
                </Row>
                <Row label="Secret backend">
                  {edit ? <window.Select value={draft.secretBackend || "env"} onChange={(e) => setDraft({ ...draft, secretBackend: e.target.value })}><option value="env">env</option><option value="aws_secretsmanager">aws_secretsmanager</option><option value="gcp_secretmanager">gcp_secretmanager</option></window.Select>
                    : <span className="mono" style={{ fontSize: 12.5 }}>{db.secretBackend}</span>}
                </Row>
                <Row label="Secret name / ARN">
                  {edit ? <window.Input mono value={draft.secretName || ""} onChange={(e) => setDraft({ ...draft, secretName: e.target.value })} />
                    : <span className="mono" style={{ fontSize: 12, color: "var(--text-2)", wordBreak: "break-all" }}>{db.secretName}</span>}
                </Row>
              </>
            )}
            <div style={{ paddingTop: 14, display: "flex", gap: 8, alignItems: "center", color: "var(--text-3)", fontSize: 12 }}>
              <Icon name="clock" size={13} /> Changes take effect on the next connection attempt — no live migration.
            </div>
          </div>
        </div>

        <div className="card">
          <div className="card-head"><div className="col" style={{ gap: 2 }}><h3>Active client connections</h3><span className="sub">Live Subscribe &amp; Listen counts</span></div><span className="badge badge-neutral" style={{ marginLeft: "auto" }}>V2</span></div>
          <div className="card-pad">
            <div className="row" style={{ alignItems: "baseline", gap: 8 }}>
              <span className="tnum" style={{ fontSize: 38, fontWeight: 600, color: "var(--text-3)" }}>&mdash;</span>
              <span style={{ color: "var(--text-3)", fontSize: 13 }}>total streams</span>
            </div>
            <div className="divider" style={{ margin: "16px 0" }} />
            <div className="col" style={{ gap: 12 }}>
              <div className="row"><Icon name="activity" size={15} style={{ color: "var(--text-3)" }} /><span style={{ fontSize: 13, color: "var(--text-2)", whiteSpace: "nowrap" }}>Subscribe <span className="faint" style={{ fontSize: 11 }}>gRPC</span></span><span className="spacer" /><b className="tnum" style={{ color: "var(--text-3)" }}>&mdash;</b></div>
              <div className="row"><Icon name="bell" size={15} style={{ color: "var(--text-3)" }} /><span style={{ fontSize: 13, color: "var(--text-2)", whiteSpace: "nowrap" }}>Listen</span><span className="spacer" /><b className="tnum" style={{ color: "var(--text-3)" }}>&mdash;</b></div>
            </div>
            <div style={{ marginTop: 16, fontSize: 11.5, color: "var(--text-3)", display: "flex", gap: 7, alignItems: "flex-start", lineHeight: 1.5 }}>
              <Icon name="clock" size={13} style={{ marginTop: 1, flexShrink: 0 }} /> Deferred to V2 — requires a shared store (Redis or a Postgres heartbeat table) to aggregate counts across embyr-server pods.
            </div>
          </div>
        </div>
      </div>
    );
  }

  /* ============ SDK API Keys ============ */
  function DbKeys({ db }) {
    const { sdkKeys, addSdkKey, revokeSdkKey } = window.useApp();
    const toast = window.useToast();
    const keys = sdkKeys[db.id] || [];
    const [createOpen, setCreateOpen] = useState(false);
    const [revealed, setRevealed] = useState(null); // {name, full}
    const [name, setName] = useState("");
    const [err, setErr] = useState("");
    const [confirmRevoke, setConfirmRevoke] = useState(null);

    function create() {
      if (!name.trim()) { setErr("Name is required."); return; }
      const k = addSdkKey(db.id, name.trim());
      setRevealed(k); setCreateOpen(false); setName(""); setErr("");
    }

    return (
      <div className="fade-in">
        <div className="row" style={{ marginBottom: 14 }}>
          <div className="col" style={{ gap: 2 }}>
            <span style={{ fontWeight: 600, fontSize: 14 }}>SDK API keys</span>
            <span style={{ fontSize: 12.5, color: "var(--text-3)" }}>Bearer tokens authorizing Firestore SDK clients to this database</span>
          </div>
          <div className="spacer" />
          <window.Button variant="primary" size="sm" icon="plus" onClick={() => { setCreateOpen(true); setName(""); setErr(""); }}>New SDK key</window.Button>
        </div>

        {keys.length === 0 ? (
          <div className="card"><div className="empty"><div className="empty-ico"><Icon name="key" size={20} /></div>No SDK keys yet.<br /><span style={{ fontSize: 12.5 }}>Create one to let SDK clients connect to <b className="mono">{db.name}</b>.</span></div></div>
        ) : (
          <div className="tbl-wrap">
            <table className="tbl">
              <thead><tr><th>Name</th><th>Prefix</th><th>Created</th><th>Last used</th><th></th></tr></thead>
              <tbody>
                {keys.map((k) => (
                  <tr key={k.id}>
                    <td style={{ fontWeight: 550 }}>{k.name}</td>
                    <td><span className="tag">{k.prefix}…</span></td>
                    <td className="td-dim">{window.fmtDate(k.created)}</td>
                    <td className="td-dim">{window.relTime(k.lastUsed)}</td>
                    <td className="td-actions">
                      <window.Button size="sm" variant="ghost" className="btn-danger" onClick={() => setConfirmRevoke(k)} style={{ height: 28 }}>Revoke</window.Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {createOpen && (
          <window.Modal title="Create SDK API key" icon="key" desc={`Scoped to ${db.name} only. No collection-level restrictions.`} onClose={() => setCreateOpen(false)}
            footer={<><window.Button variant="ghost" onClick={() => setCreateOpen(false)}>Cancel</window.Button><window.Button variant="primary" onClick={create} icon="plus">Create key</window.Button></>}>
            <window.Field label="Name" error={err} hint={!err && "A human label, e.g. web-client or ios-app."}>
              <window.Input value={name} autoFocus placeholder="web-client" error={err} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && create()} />
            </window.Field>
          </window.Modal>
        )}

        {revealed && <window.RevealKeyModal name={revealed.name} value={revealed.full} kind="SDK" onClose={() => { setRevealed(null); toast({ msg: "SDK key created", icon: "key" }); }} />}

        {confirmRevoke && (
          <window.Modal title={`Revoke “${confirmRevoke.name}”?`} icon="key" onClose={() => setConfirmRevoke(null)}
            desc="In-flight requests using this key are rejected on next token validation. This cannot be undone."
            footer={<><window.Button variant="ghost" onClick={() => setConfirmRevoke(null)}>Cancel</window.Button><window.Button variant="danger" onClick={() => { revokeSdkKey(db.id, confirmRevoke.id); setConfirmRevoke(null); toast({ msg: "SDK key revoked", icon: "key", tone: "red" }); }}>Revoke key</window.Button></>} />
        )}
      </div>
    );
  }

  /* ============ wrapper ============ */
  function Wrapper() {
    const { currentDb, nav, openDb, setDbStatus, deleteDatabase, go } = window.useApp();
    const toast = window.useToast();
    const [confirmDel, setConfirmDel] = useState(false);
    const db = currentDb;
    if (!db) { go("databases"); return null; }
    const sub = nav.sub || "overview";

    const tabs = [
      { value: "overview", label: "Overview", icon: "gauge" },
      { value: "connections", label: "Connections", icon: "link" },
      { value: "logs", label: "Query Logs", icon: "list" },
      { value: "keys", label: "API Keys", icon: "key", count: (window.useApp().sdkKeys[db.id] || []).length },
    ];

    return (
      <div className="page page-wide">
        <div className="page-head" style={{ marginBottom: 18 }}>
          <button className="btn btn-ghost btn-icon-only" onClick={() => go("databases")} title="Back to databases" style={{ marginTop: 2 }}><Icon name="arrowLeft" size={17} /></button>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className="row" style={{ gap: 11 }}>
              <h1 className="page-title mono" style={{ letterSpacing: "-0.02em", whiteSpace: "nowrap" }}>{db.name}</h1>
              <window.StatusBadge status={db.status} />
            </div>
            <p className="page-sub">{db.region} · <span className="mono">{db.backendMode}</span> · created {window.fmtDate(db.created)}</p>
          </div>
          <div className="page-head-actions">
            {db.status === "active"
              ? <window.Button variant="ghost" icon="pause" onClick={() => { setDbStatus(db.id, "suspended"); toast({ msg: `${db.name} suspended`, icon: "pause", tone: "amber" }); }}>Suspend</window.Button>
              : <window.Button variant="ghost" icon="play" onClick={() => { setDbStatus(db.id, "active"); toast({ msg: `${db.name} activated`, icon: "play" }); }}>Activate</window.Button>}
            <window.Menu trigger={<button className="btn btn-icon-only"><Icon name="more" size={17} /></button>}>
              <window.MenuItem icon="copy" onClick={() => navigator.clipboard?.writeText(db.id)}>Copy database ID</window.MenuItem>
              <div className="menu-sep" />
              <window.MenuItem icon="trash" danger onClick={() => setConfirmDel(true)}>Delete database</window.MenuItem>
            </window.Menu>
          </div>
        </div>

        <window.Tabs value={sub} onChange={(v) => openDb(db.id, v)} tabs={tabs} />

        {sub === "overview" && <DbOverview db={db} />}
        {sub === "connections" && <DbConnections db={db} />}
        {sub === "logs" && <window.DbLogs db={db} />}
        {sub === "keys" && <DbKeys db={db} />}

        {confirmDel && (
          <window.Modal title={`Delete ${db.name}?`} icon="trash" onClose={() => setConfirmDel(false)}
            desc="This permanently deletes the database and revokes all its SDK API keys. This cannot be undone."
            footer={<><window.Button variant="ghost" onClick={() => setConfirmDel(false)}>Cancel</window.Button><window.Button variant="danger" icon="trash" onClick={() => { const n = db.name; deleteDatabase(db.id); go("databases"); toast({ msg: `${n} deleted`, icon: "trash", tone: "red" }); }}>Delete database</window.Button></>} />
        )}
      </div>
    );
  }

  return Wrapper;
})();
window.DatabaseDetail = DatabaseDetail;
