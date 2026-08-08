/* embyr console — Databases list + Create modal */
const DatabasesView = (function () {
  const { useState, useMemo } = React;
  const Icon = window.Icon;

  function CreateDatabaseModal({ onClose }) {
    const { createDatabase, openDb, databases } = window.useApp();
    const toast = window.useToast();
    const [name, setName] = useState("");
    const [mode, setMode] = useState("direct_pg");
    const [secretBackend, setSecretBackend] = useState("env");
    const [url, setUrl] = useState("");
    const [endpoint, setEndpoint] = useState("");
    const [secretName, setSecretName] = useState("");
    const [region, setRegion] = useState("us-east-1");
    const [err, setErr] = useState({});

    function submit() {
      const e = {};
      if (!name.trim()) e.name = "Name is required.";
      else if (!/^[a-z0-9][a-z0-9-]{1,38}$/.test(name)) e.name = "Lowercase letters, digits and hyphens; 2–39 chars.";
      else if (databases.some((d) => d.name === name)) e.name = "A database with this name already exists.";
      if (mode === "direct_pg" && !url.trim()) e.url = "Connection URL is required.";
      if (mode === "agent_mode" && !endpoint.trim()) e.endpoint = "Agent endpoint is required.";
      if (mode === "agent_mode" && secretBackend !== "env" && !secretName.trim()) e.secretName = "Secret name / ARN is required.";
      setErr(e);
      if (Object.keys(e).length) return;
      const db = createDatabase({ name, backendMode: mode, secretBackend: mode === "agent_mode" ? secretBackend : null, connectionUrl: url, agentEndpoint: endpoint, secretName, region });
      toast({ msg: `Database “${name}” created`, icon: "database" });
      onClose();
      openDb(db.id);
    }

    return (
      <window.Modal title="Create database" desc="A database maps to one Postgres-backed project. The name is immutable once created." icon="database" onClose={onClose} size="lg"
        footer={<><window.Button variant="ghost" onClick={onClose}>Cancel</window.Button><window.Button variant="primary" onClick={submit} icon="plus">Create database</window.Button></>}>
        <window.Field label="Name" error={err.name} hint={!err.name && "Used in API endpoints. Cannot be changed later."}>
          <window.Input mono value={name} placeholder="my-database" autoFocus error={err.name} onChange={(e) => setName(e.target.value)} />
        </window.Field>

        <window.Field label="Backend mode">
          <window.Segmented value={mode} onChange={setMode} options={[
            { value: "direct_pg", label: "Direct Postgres", icon: "database" },
            { value: "agent_mode", label: "Agent (mTLS)", icon: "server" },
          ]} />
          <span className="field-hint">{mode === "direct_pg"
            ? "embyr connects straight to your Postgres DSN."
            : "embyr talks to an agent over mTLS gRPC; credentials live in a secret backend."}</span>
        </window.Field>

        {mode === "direct_pg" ? (
          <window.Field label="Connection URL" error={err.url} hint={!err.url && "Credentials are stored encrypted and masked in the UI."}>
            <window.Input mono value={url} placeholder="host:5432/dbname" error={err.url} onChange={(e) => setUrl(e.target.value)} />
          </window.Field>
        ) : (
          <>
            <window.Field label="Agent endpoint" error={err.endpoint}>
              <window.Input mono value={endpoint} placeholder="agent.host:8443" error={err.endpoint} onChange={(e) => setEndpoint(e.target.value)} />
            </window.Field>
            <div className="grid" style={{ gridTemplateColumns: "1fr 1.4fr", gap: 14 }}>
              <window.Field label="Secret backend">
                <window.Select value={secretBackend} onChange={(e) => setSecretBackend(e.target.value)}>
                  <option value="env">Environment</option>
                  <option value="aws_secretsmanager">AWS Secrets Manager</option>
                  <option value="gcp_secretmanager">GCP Secret Manager</option>
                </window.Select>
              </window.Field>
              <window.Field label={secretBackend === "aws_secretsmanager" ? "Secret ARN" : "Secret name"} error={err.secretName}>
                <window.Input mono disabled={secretBackend === "env"} value={secretName} placeholder={secretBackend === "env" ? "n/a for env" : "embyr/prod-credentials"} error={err.secretName} onChange={(e) => setSecretName(e.target.value)} />
              </window.Field>
            </div>
          </>
        )}

        <window.Field label="Region">
          <window.Select value={region} onChange={(e) => setRegion(e.target.value)}>
            <option>us-east-1</option><option>us-west-2</option><option>europe-west1</option><option>ap-southeast-1</option>
          </window.Select>
        </window.Field>
      </window.Modal>
    );
  }
  window.CreateDatabaseModal = CreateDatabaseModal;

  const MODE_LABEL = { direct_pg: "Direct Postgres", agent_mode: "Agent (mTLS)" };

  function DatabasesList() {
    const { databases, openDb, setDbStatus, deleteDatabase, fmtNum } = window.useApp();
    const toast = window.useToast();
    const [q, setQ] = useState("");
    const [view, setView] = useState("table");
    const [newOpen, setNewOpen] = useState(false);
    const [confirmDel, setConfirmDel] = useState(null);

    const rows = useMemo(() => databases.filter((d) => d.name.toLowerCase().includes(q.toLowerCase())), [databases, q]);

    return (
      <div className="page page-wide">
        <div className="page-head">
          <div>
            <h1 className="page-title">Databases</h1>
            <p className="page-sub">{databases.length} databases in this account</p>
          </div>
          <div className="page-head-actions">
            <window.Button variant="primary" icon="plus" onClick={() => setNewOpen(true)}>New database</window.Button>
          </div>
        </div>

        <div className="row" style={{ marginBottom: 16 }}>
          <div style={{ width: 280 }}><window.Input icon="search" placeholder="Search databases…" value={q} onChange={(e) => setQ(e.target.value)} /></div>
          <div className="spacer" />
          <window.Segmented value={view} onChange={setView} options={[{ value: "table", icon: "list", label: "" }, { value: "grid", icon: "grid", label: "" }]} />
        </div>

        {view === "grid" ? (
          <div className="grid" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(300px, 1fr))" }}>
            {rows.map((db) => <DbGridCard key={db.id} db={db} />)}
          </div>
        ) : (
          <div className="tbl-wrap">
            <table className="tbl">
              <thead><tr>
                <th>Name</th><th>Status</th><th>Backend</th><th className="td-right">P95</th><th>Created</th><th></th>
              </tr></thead>
              <tbody>
                {rows.map((db) => (
                  <tr key={db.id} className="clickable" onClick={() => openDb(db.id)}>
                    <td>
                      <div className="row" style={{ gap: 10 }}>
                        <span style={{ width: 26, height: 26, borderRadius: 7, background: "var(--accent-soft)", color: "var(--accent)", display: "grid", placeItems: "center" }}><Icon name="database" size={13} /></span>
                        <div className="col" style={{ gap: 1 }}>
                          <span style={{ fontWeight: 550, whiteSpace: "nowrap" }}>{db.name}</span>
                          <span className="mono" style={{ fontSize: 11, color: "var(--text-3)", whiteSpace: "nowrap" }}>{db.region}</span>
                        </div>
                      </div>
                    </td>
                    <td><window.StatusBadge status={db.status} /></td>
                    <td><span className="badge badge-neutral">{MODE_LABEL[db.backendMode]}</span></td>
                    <td className="td-right tnum">{db.status === "active" ? db.p95 + " ms" : "—"}</td>
                    <td className="td-dim">{window.fmtDate(db.created)}</td>
                    <td className="td-actions" onClick={(e) => e.stopPropagation()}>
                      <window.Menu trigger={<button className="iconbtn"><Icon name="more" size={17} /></button>}>
                        <window.MenuItem icon="arrowUpRight" onClick={() => openDb(db.id)}>Open</window.MenuItem>
                        {db.status === "active"
                          ? <window.MenuItem icon="pause" onClick={() => { setDbStatus(db.id, "suspended"); toast({ msg: `${db.name} suspended`, icon: "pause", tone: "amber" }); }}>Suspend</window.MenuItem>
                          : <window.MenuItem icon="play" onClick={() => { setDbStatus(db.id, "active"); toast({ msg: `${db.name} activated`, icon: "play" }); }}>Activate</window.MenuItem>}
                        <div className="menu-sep" />
                        <window.MenuItem icon="trash" danger onClick={() => setConfirmDel(db)}>Delete</window.MenuItem>
                      </window.Menu>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {newOpen && <CreateDatabaseModal onClose={() => setNewOpen(false)} />}
        {confirmDel && (
          <window.Modal title={`Delete ${confirmDel.name}?`} icon="trash" onClose={() => setConfirmDel(null)}
            desc="This permanently deletes the database and cascades to revoke all its SDK API keys. This cannot be undone."
            footer={<><window.Button variant="ghost" onClick={() => setConfirmDel(null)}>Cancel</window.Button>
              <window.Button variant="danger" icon="trash" onClick={() => { deleteDatabase(confirmDel.id); toast({ msg: `${confirmDel.name} deleted`, icon: "trash", tone: "red" }); setConfirmDel(null); }}>Delete database</window.Button></>}>
            <div className="copyfield" style={{ borderColor: "var(--red)", color: "var(--red)" }}>
              <Icon name="alert" size={15} /> All SDK keys for this database will stop working immediately.
            </div>
          </window.Modal>
        )}
      </div>
    );
  }

  function DbGridCard({ db }) {
    const { openDb, fmtNum } = window.useApp();
    return (
      <button className="card" onClick={() => openDb(db.id)} style={{ textAlign: "left", cursor: "pointer", padding: 16 }}>
        <div className="row" style={{ marginBottom: 12 }}>
          <span style={{ width: 28, height: 28, borderRadius: 7, background: "var(--accent-soft)", color: "var(--accent)", display: "grid", placeItems: "center" }}><Icon name="database" size={14} /></span>
          <span style={{ fontWeight: 600 }}>{db.name}</span>
          <div className="spacer" /><window.StatusBadge status={db.status} />
        </div>
        <window.Sparkline data={db.latency} w={280} h={36} color="var(--accent)" />
        <div className="row" style={{ marginTop: 12, gap: 18, fontSize: 12, color: "var(--text-2)" }}>
          <span>P95 <span className="tnum" style={{ color: "var(--text)", fontWeight: 600 }}>{db.status === "active" ? db.p95 + "ms" : "—"}</span></span>
          <span>Backend <span className="mono" style={{ color: "var(--text-2)", fontWeight: 500, fontSize: 11.5 }}>{db.backendMode}</span></span>
        </div>
      </button>
    );
  }

  return DatabasesList;
})();
window.DatabasesView = DatabasesView;
