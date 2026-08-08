/* embyr console — Account Settings */
const SettingsView = (function () {
  const { useState } = React;
  const Icon = window.Icon;

  function SectionCard({ title, sub, children, foot }) {
    return (
      <div className="card" style={{ marginBottom: 18 }}>
        <div className="card-head"><div className="col" style={{ gap: 2 }}><h3>{title}</h3>{sub && <span className="sub">{sub}</span>}</div></div>
        <div className="card-pad">{children}</div>
        {foot && <div style={{ padding: "12px 20px", borderTop: "1px solid var(--border)", display: "flex", justifyContent: "flex-end", gap: 10, background: "var(--bg-elev)", borderRadius: "0 0 var(--r) var(--r)" }}>{foot}</div>}
      </div>
    );
  }

  function Settings() {
    const { account, oidcProviders, toggleOidc, user } = window.useApp();
    const toast = window.useToast();
    const [name, setName] = useState(account.name);
    const [showRecovery, setShowRecovery] = useState(false);
    const [confirmDelete, setConfirmDelete] = useState(false);
    const [delText, setDelText] = useState("");

    const recoveryCodes = ["7f2a-91bd", "3c08-44ee", "a190-2b7c", "55d1-0f9a", "e8c4-7710", "2bb9-63da", "9047-1ace", "df3e-8052"];

    return (
      <div className="page">
        <div className="page-head"><div><h1 className="page-title">Account Settings</h1><p className="page-sub">Manage account identity, authentication, and security</p></div></div>

        <SectionCard title="Account" sub="Display name and identifiers"
          foot={<window.Button variant="primary" size="sm" disabled={name === account.name} onClick={() => toast({ msg: "Account name updated", icon: "check" })}>Save changes</window.Button>}>
          <div className="grid" style={{ gridTemplateColumns: "1fr 1fr", gap: 18 }}>
            <window.Field label="Account name"><window.Input value={name} onChange={(e) => setName(e.target.value)} /></window.Field>
            <window.Field label="Account ID"><div className="copyfield" style={{ background: "var(--surface-2)" }}>{account.id}<span className="spacer" /><window.CopyInline value={account.id}></window.CopyInline></div></window.Field>
          </div>
        </SectionCard>

        <SectionCard title="Authentication methods" sub="Both methods can be enabled simultaneously for this account">
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <div className="row" style={{ padding: "13px 15px", border: "1px solid var(--border)", borderRadius: "var(--r)", gap: 13 }}>
              <span style={{ width: 34, height: 34, borderRadius: 9, background: "var(--surface-2)", display: "grid", placeItems: "center" }}><Icon name="mail" size={16} /></span>
              <div className="col" style={{ gap: 1, flex: 1 }}>
                <span style={{ fontWeight: 550, fontSize: 13.5 }}>Email + Password + MFA</span>
                <span style={{ fontSize: 12, color: "var(--text-3)" }}>Argon2id hashing · MFA mandatory (TOTP or Email OTP)</span>
              </div>
              <window.Badge tone="green" dot>enabled</window.Badge>
            </div>

            <div className="col" style={{ gap: 0, border: "1px solid var(--border)", borderRadius: "var(--r)", overflow: "hidden" }}>
              <div className="row" style={{ padding: "13px 15px", gap: 13 }}>
                <span style={{ width: 34, height: 34, borderRadius: 9, background: "var(--surface-2)", display: "grid", placeItems: "center" }}><Icon name="globe" size={16} /></span>
                <div className="col" style={{ gap: 1, flex: 1 }}>
                  <span style={{ fontWeight: 550, fontSize: 13.5 }}>OIDC providers</span>
                  <span style={{ fontSize: 12, color: "var(--text-3)" }}>Owner-only · single sign-on via trusted identity providers</span>
                </div>
                <window.Button size="sm" variant="ghost" icon="plus" onClick={() => toast({ msg: "Add provider — configure issuer & client", icon: "globe" })}>Add provider</window.Button>
              </div>
              {oidcProviders.map((p) => (
                <div key={p.id} className="row" style={{ padding: "11px 15px 11px 62px", borderTop: "1px solid var(--border)", gap: 12 }}>
                  <div className="col" style={{ gap: 1, flex: 1 }}>
                    <span style={{ fontWeight: 530, fontSize: 13 }}>{p.name}</span>
                    <span className="mono" style={{ fontSize: 11, color: "var(--text-3)" }}>{p.issuer}</span>
                  </div>
                  <window.Toggle on={p.enabled} onChange={() => { toggleOidc(p.id); toast({ msg: `${p.name} ${p.enabled ? "disabled" : "enabled"}`, icon: "globe", tone: p.enabled ? "amber" : "green" }); }} />
                </div>
              ))}
            </div>
          </div>
        </SectionCard>

        <SectionCard title="Your security" sub="Second factor and recovery for your sign-in">
          <div className="grid" style={{ gridTemplateColumns: "1fr 1fr", gap: 14 }}>
            <div className="row" style={{ padding: "13px 15px", border: "1px solid var(--border)", borderRadius: "var(--r)", gap: 12 }}>
              <Icon name="shield" size={18} style={{ color: "var(--accent)" }} />
              <div className="col" style={{ gap: 1, flex: 1 }}><span style={{ fontWeight: 550, fontSize: 13 }}>TOTP authenticator</span><span style={{ fontSize: 11.5, color: "var(--text-3)" }}>Active second factor</span></div>
              <window.Badge tone="green" dot>on</window.Badge>
            </div>
            <button className="row" onClick={() => setShowRecovery(true)} style={{ padding: "13px 15px", border: "1px solid var(--border)", borderRadius: "var(--r)", gap: 12, cursor: "pointer", background: "none", textAlign: "left" }}>
              <Icon name="key" size={18} style={{ color: "var(--text-2)" }} />
              <div className="col" style={{ gap: 1, flex: 1 }}><span style={{ fontWeight: 550, fontSize: 13 }}>Recovery codes</span><span style={{ fontSize: 11.5, color: "var(--text-3)" }}>8 single-use codes · view & regenerate</span></div>
              <Icon name="chevronRight" size={16} style={{ color: "var(--text-3)" }} />
            </button>
          </div>
        </SectionCard>

        {/* DANGER ZONE — owner only */}
        <div className="card" style={{ borderColor: "color-mix(in oklab, var(--red) 30%, var(--border))" }}>
          <div className="card-head"><div className="col" style={{ gap: 2 }}><h3 style={{ color: "var(--red)" }}>Danger zone</h3><span className="sub">Owner-only, irreversible actions</span></div></div>
          <div style={{ display: "flex", flexDirection: "column" }}>
            <div className="row" style={{ padding: "15px 20px", borderBottom: "1px solid var(--border)", gap: 12 }}>
              <div className="col" style={{ gap: 2, flex: 1 }}><span style={{ fontWeight: 550, fontSize: 13.5 }}>Transfer ownership</span><span style={{ fontSize: 12, color: "var(--text-3)" }}>Hand the Owner role to another member.</span></div>
              <window.Button variant="ghost" onClick={() => toast({ msg: "Select a member to transfer to", icon: "users" })}>Transfer</window.Button>
            </div>
            <div className="row" style={{ padding: "15px 20px", gap: 12 }}>
              <div className="col" style={{ gap: 2, flex: 1 }}><span style={{ fontWeight: 550, fontSize: 13.5 }}>Delete account</span><span style={{ fontSize: 12, color: "var(--text-3)" }}>Permanently delete this account and all databases.</span></div>
              <window.Button variant="danger" icon="trash" onClick={() => { setConfirmDelete(true); setDelText(""); }}>Delete account</window.Button>
            </div>
          </div>
        </div>

        {showRecovery && (
          <window.Modal title="Recovery codes" icon="key" desc="Each code can be used once if you lose your second factor. Store them somewhere safe." onClose={() => setShowRecovery(false)}
            footer={<><window.Button variant="ghost" icon="refresh" onClick={() => toast({ msg: "New recovery codes generated", icon: "key" })}>Regenerate</window.Button><window.Button variant="primary" icon="download" onClick={() => { setShowRecovery(false); toast({ msg: "Recovery codes downloaded", icon: "download" }); }}>Download</window.Button></>}>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
              {recoveryCodes.map((c) => <div key={c} className="mono" style={{ background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 7, padding: "9px 12px", fontSize: 13, textAlign: "center", letterSpacing: "0.04em" }}>{c}</div>)}
            </div>
          </window.Modal>
        )}

        {confirmDelete && (
          <window.Modal title={`Delete ${account.name}?`} icon="alert" onClose={() => setConfirmDelete(false)}
            desc="This permanently deletes the account, every database, and all keys. There is no recovery."
            footer={<><window.Button variant="ghost" onClick={() => setConfirmDelete(false)}>Cancel</window.Button><window.Button variant="danger" icon="trash" disabled={delText !== account.name} onClick={() => { setConfirmDelete(false); toast({ msg: "Account deletion is disabled in this demo", icon: "alert", tone: "amber" }); }}>Delete forever</window.Button></>}>
            <window.Field label={<span>Type <b className="mono">{account.name}</b> to confirm</span>}>
              <window.Input className="input-err" value={delText} onChange={(e) => setDelText(e.target.value)} placeholder={account.name} />
            </window.Field>
          </window.Modal>
        )}
      </div>
    );
  }
  return Settings;
})();
window.SettingsView = SettingsView;
