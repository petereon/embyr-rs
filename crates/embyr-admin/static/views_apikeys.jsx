/* embyr console — Admin API Keys (account level) */
const ApiKeysView = (function () {
  const { useState } = React;
  const Icon = window.Icon;

  function AdminKeys() {
    const { adminKeys, addAdminKey, revokeAdminKey, members, serviceAccounts } = window.useApp();
    const toast = window.useToast();
    const [open, setOpen] = useState(false);
    const [name, setName] = useState("");
    const [identityKey, setIdentityKey] = useState("");
    const [err, setErr] = useState({});
    const [revealed, setRevealed] = useState(null);
    const [confirmRevoke, setConfirmRevoke] = useState(null);

    const identities = [
      ...members.filter((m) => !m.pending).map((m) => ({ key: "m:" + m.id, label: m.name, type: "member", role: m.role, icon: "users" })),
      ...serviceAccounts.map((s) => ({ key: "s:" + s.id, label: s.name, type: "service", role: s.role, icon: "bot" })),
    ];
    const chosen = identities.find((i) => i.key === identityKey);

    function create() {
      const e = {};
      if (!name.trim()) e.name = "Name is required.";
      if (!chosen) e.identity = "Select an identity.";
      setErr(e); if (Object.keys(e).length) return;
      const k = addAdminKey({ name, identity: chosen.label, identityType: chosen.type, role: chosen.role });
      setRevealed(k); setOpen(false); setName(""); setIdentityKey(""); setErr({});
    }

    return (
      <div className="page page-wide">
        <div className="page-head">
          <div><h1 className="page-title">API Keys</h1><p className="page-sub">Account-level admin keys for programmatic management — create databases, manage members, and more</p></div>
          <div className="page-head-actions"><window.Button variant="primary" icon="plus" onClick={() => { setOpen(true); setErr({}); }}>New admin key</window.Button></div>
        </div>

        <div className="card card-pad" style={{ display: "flex", gap: 12, alignItems: "center", marginBottom: 18, background: "var(--accent-soft)", borderColor: "var(--accent-line)" }}>
          <Icon name="shield" size={18} style={{ color: "var(--accent)" }} />
          <span style={{ fontSize: 13, color: "var(--text)" }}>Admin keys are prefixed <span className="mono" style={{ background: "var(--surface)", padding: "1px 6px", borderRadius: 5 }}>embyr_adm_</span> and carry the role of their associated identity. Keep them in a secret manager.</span>
        </div>

        {adminKeys.length === 0 ? (
          <div className="card"><div className="empty"><div className="empty-ico"><Icon name="key" size={20} /></div>No admin keys yet.</div></div>
        ) : (
          <div className="tbl-wrap">
            <table className="tbl">
              <thead><tr><th>Name</th><th>Associated identity</th><th>Role</th><th>Prefix</th><th>Created</th><th>Last used</th><th></th></tr></thead>
              <tbody>
                {adminKeys.map((k) => (
                  <tr key={k.id}>
                    <td style={{ fontWeight: 550 }}>{k.name}</td>
                    <td><span className="row" style={{ gap: 8 }}><Icon name={k.identityType === "service" ? "bot" : "users"} size={14} style={{ color: "var(--text-3)" }} /><span className="mono" style={{ fontSize: 12.5, whiteSpace: "nowrap" }}>{k.identity}</span></span></td>
                    <td><window.RoleBadge role={k.role} /></td>
                    <td><span className="tag">{k.prefix}…</span></td>
                    <td className="td-dim">{window.fmtDate(k.created)}</td>
                    <td className="td-dim">{window.relTime(k.lastUsed)}</td>
                    <td className="td-actions"><window.Button size="sm" variant="ghost" className="btn-danger" style={{ height: 28 }} onClick={() => setConfirmRevoke(k)}>Revoke</window.Button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {open && (
          <window.Modal title="Create admin API key" icon="key" desc="The key inherits the role of the identity you associate it with." onClose={() => setOpen(false)}
            footer={<><window.Button variant="ghost" onClick={() => setOpen(false)}>Cancel</window.Button><window.Button variant="primary" icon="plus" onClick={create}>Create key</window.Button></>}>
            <window.Field label="Associated identity" error={err.identity}>
              <window.Select value={identityKey} error={err.identity} onChange={(e) => setIdentityKey(e.target.value)}>
                <option value="" disabled>Select a member or service account…</option>
                <optgroup label="Members">{identities.filter((i) => i.type === "member").map((i) => <option key={i.key} value={i.key}>{i.label} · {i.role}</option>)}</optgroup>
                <optgroup label="Service accounts">{identities.filter((i) => i.type === "service").map((i) => <option key={i.key} value={i.key}>{i.label} · {i.role}</option>)}</optgroup>
              </window.Select>
            </window.Field>
            {chosen && <div className="row" style={{ gap: 8, fontSize: 12.5, color: "var(--text-2)" }}><Icon name="shield" size={14} style={{ color: "var(--text-3)" }} />Role <window.RoleBadge role={chosen.role} /> is inherited from {chosen.label}.</div>}
            <window.Field label="Name" error={err.name} hint={!err.name && "A human label to recognize this key."}>
              <window.Input value={name} placeholder="terraform" error={err.name} onChange={(e) => setName(e.target.value)} />
            </window.Field>
          </window.Modal>
        )}

        {revealed && <window.RevealKeyModal name={revealed.name} value={revealed.full} kind="Admin" onClose={() => { setRevealed(null); toast({ msg: "Admin key created", icon: "key" }); }} />}
        {confirmRevoke && (
          <window.Modal title={`Revoke “${confirmRevoke.name}”?`} icon="key" onClose={() => setConfirmRevoke(null)}
            desc="Programmatic access using this key stops immediately. This cannot be undone."
            footer={<><window.Button variant="ghost" onClick={() => setConfirmRevoke(null)}>Cancel</window.Button><window.Button variant="danger" onClick={() => { revokeAdminKey(confirmRevoke.id); setConfirmRevoke(null); toast({ msg: "Admin key revoked", icon: "key", tone: "red" }); }}>Revoke key</window.Button></>} />
        )}
      </div>
    );
  }
  return AdminKeys;
})();
window.ApiKeysView = ApiKeysView;
