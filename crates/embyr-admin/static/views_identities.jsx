/* embyr console — Identities: Members + Service Accounts */
const IdentitiesView = (function () {
  const { useState } = React;
  const Icon = window.Icon;

  const AUTH_LABEL = { oidc: "OIDC", email_password: "Email + Password", "—": "—" };

  function Members() {
    const { members, inviteMember, setMemberRole, removeMember } = window.useApp();
    const toast = window.useToast();
    const [inviteOpen, setInviteOpen] = useState(false);
    const [email, setEmail] = useState(""); const [role, setRole] = useState("viewer"); const [err, setErr] = useState("");
    const [confirmRemove, setConfirmRemove] = useState(null);

    const ownerCount = members.filter((m) => m.role === "owner" && !m.pending).length;
    function isLastOwner(m) { return m.role === "owner" && ownerCount <= 1; }

    function invite() {
      if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) { setErr("Enter a valid email address."); return; }
      if (members.some((m) => m.email === email)) { setErr("This person is already a member or invited."); return; }
      inviteMember(email, role); toast({ msg: `Invitation sent to ${email}`, icon: "mail" });
      setInviteOpen(false); setEmail(""); setRole("viewer"); setErr("");
    }

    return (
      <div className="fade-in">
        <div className="row" style={{ marginBottom: 14 }}>
          <span style={{ fontSize: 12.5, color: "var(--text-3)" }}>{members.filter((m) => !m.pending).length} members · {members.filter((m) => m.pending).length} pending invites</span>
          <div className="spacer" />
          <window.Button variant="primary" size="sm" icon="plus" onClick={() => { setInviteOpen(true); setErr(""); }}>Invite member</window.Button>
        </div>

        <div className="tbl-wrap">
          <table className="tbl">
            <thead><tr><th>Member</th><th>Role</th><th>Auth method</th><th>MFA</th><th>Last login</th><th></th></tr></thead>
            <tbody>
              {members.map((m) => (
                <tr key={m.id}>
                  <td>
                    <div className="row" style={{ gap: 11 }}>
                      <window.Avatar name={m.pending ? m.email : m.name} size={30} />
                      <div className="col" style={{ gap: 1, minWidth: 0 }}>
                        <span style={{ fontWeight: 550, whiteSpace: "nowrap" }}>{m.pending ? m.email : m.name}{m.you && <span className="tag" style={{ marginLeft: 7, height: 18 }}>you</span>}</span>
                        {!m.pending && <span style={{ fontSize: 11.5, color: "var(--text-3)", whiteSpace: "nowrap" }}>{m.email}</span>}
                      </div>
                    </div>
                  </td>
                  <td>{m.pending ? <window.Badge tone="blue" dot>pending</window.Badge> : <window.RoleBadge role={m.role} />}</td>
                  <td className="td-dim">{m.pending ? "—" : AUTH_LABEL[m.auth]}</td>
                  <td>{m.pending || m.mfa === "—" ? <span className="td-dim">—</span> : <span className="badge badge-neutral"><Icon name="shield" size={11} />{m.mfa}</span>}</td>
                  <td className="td-dim">{m.pending ? <span style={{ color: "var(--blue)" }}>invite sent</span> : window.relTime(m.lastLogin)}</td>
                  <td className="td-actions">
                    <window.Menu trigger={<button className="iconbtn"><Icon name="more" size={17} /></button>} width={188}>
                      {!m.pending && !m.you && (
                        <>
                          <div className="menu-label">Change role</div>
                          {["owner", "admin", "viewer"].map((r) => (
                            <window.MenuItem key={r} icon={m.role === r ? "check" : undefined} onClick={() => { setMemberRole(m.id, r); toast({ msg: `${m.name} is now ${r}`, icon: "shield" }); }}>
                              <span style={{ textTransform: "capitalize", paddingLeft: m.role === r ? 0 : 25 }}>{r}</span>
                            </window.MenuItem>
                          ))}
                          <div className="menu-sep" />
                        </>
                      )}
                      {m.pending && <window.MenuItem icon="mail" onClick={() => toast({ msg: "Invitation resent", icon: "mail" })}>Resend invite</window.MenuItem>}
                      <window.MenuItem icon="trash" danger disabled={isLastOwner(m)}
                        onClick={() => isLastOwner(m) ? toast({ msg: "Can't remove the last owner", icon: "alert", tone: "amber" }) : setConfirmRemove(m)}>
                        {m.pending ? "Revoke invite" : m.you ? "Leave account" : "Remove"}
                      </window.MenuItem>
                    </window.Menu>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {inviteOpen && (
          <window.Modal title="Invite member" icon="mail" desc="They'll receive an email to set up credentials and join. Invitations expire after 7 days." onClose={() => setInviteOpen(false)}
            footer={<><window.Button variant="ghost" onClick={() => setInviteOpen(false)}>Cancel</window.Button><window.Button variant="primary" icon="mail" onClick={invite}>Send invitation</window.Button></>}>
            <window.Field label="Email address" error={err}>
              <window.Input type="email" autoFocus placeholder="teammate@company.com" value={email} error={err} onChange={(e) => setEmail(e.target.value)} onKeyDown={(e) => e.key === "Enter" && invite()} />
            </window.Field>
            <window.Field label="Role" hint="Owners can configure OIDC, delete the account, and transfer ownership.">
              <window.Select value={role} onChange={(e) => setRole(e.target.value)}><option value="viewer">Viewer</option><option value="admin">Admin</option><option value="owner">Owner</option></window.Select>
            </window.Field>
          </window.Modal>
        )}

        {confirmRemove && (
          <window.Modal title={`Remove ${confirmRemove.name === "—" ? confirmRemove.email : confirmRemove.name}?`} icon="trash" onClose={() => setConfirmRemove(null)}
            desc="This immediately revokes their session and access. Any admin API keys they hold are invalidated."
            footer={<><window.Button variant="ghost" onClick={() => setConfirmRemove(null)}>Cancel</window.Button><window.Button variant="danger" icon="trash" onClick={() => { removeMember(confirmRemove.id); setConfirmRemove(null); toast({ msg: "Member removed", icon: "trash", tone: "red" }); }}>Remove</window.Button></>} />
        )}
      </div>
    );
  }

  function ServiceAccounts() {
    const { serviceAccounts, createServiceAccount, deleteServiceAccount } = window.useApp();
    const toast = window.useToast();
    const [open, setOpen] = useState(false);
    const [form, setForm] = useState({ name: "", description: "", role: "viewer" });
    const [err, setErr] = useState("");
    const [confirmDel, setConfirmDel] = useState(null);

    function create() {
      if (!form.name.trim()) { setErr("Name is required."); return; }
      createServiceAccount(form); toast({ msg: `Service account “${form.name}” created`, icon: "bot" });
      setOpen(false); setForm({ name: "", description: "", role: "viewer" }); setErr("");
    }

    return (
      <div className="fade-in">
        <div className="row" style={{ marginBottom: 14 }}>
          <span style={{ fontSize: 12.5, color: "var(--text-3)" }}>{serviceAccounts.length} non-human identities · credentials are admin API keys created separately</span>
          <div className="spacer" />
          <window.Button variant="primary" size="sm" icon="plus" onClick={() => { setOpen(true); setErr(""); }}>New service account</window.Button>
        </div>

        <div className="tbl-wrap">
          <table className="tbl">
            <thead><tr><th>Name</th><th>Description</th><th>Role</th><th>Created</th><th>Last used</th><th></th></tr></thead>
            <tbody>
              {serviceAccounts.map((s) => (
                <tr key={s.id}>
                  <td><div className="row" style={{ gap: 10 }}><span style={{ width: 28, height: 28, borderRadius: 7, background: "var(--surface-2)", color: "var(--text-2)", display: "grid", placeItems: "center" }}><Icon name="bot" size={15} /></span><span className="mono" style={{ fontWeight: 550, fontSize: 12.5 }}>{s.name}</span></div></td>
                  <td className="td-dim">{s.description}</td>
                  <td><window.RoleBadge role={s.role} /></td>
                  <td className="td-dim">{window.fmtDate(s.created)}</td>
                  <td className="td-dim">{window.relTime(s.lastUsed)}</td>
                  <td className="td-actions">
                    <window.Menu trigger={<button className="iconbtn"><Icon name="more" size={17} /></button>}>
                      <window.MenuItem icon="key" onClick={() => toast({ msg: "Create keys in the API Keys section", icon: "key" })}>Manage keys</window.MenuItem>
                      <div className="menu-sep" />
                      <window.MenuItem icon="trash" danger onClick={() => setConfirmDel(s)}>Delete</window.MenuItem>
                    </window.Menu>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {open && (
          <window.Modal title="New service account" icon="bot" desc="A non-human identity. It can't log in interactively — attach admin API keys to it in the API Keys section." onClose={() => setOpen(false)}
            footer={<><window.Button variant="ghost" onClick={() => setOpen(false)}>Cancel</window.Button><window.Button variant="primary" icon="plus" onClick={create}>Create</window.Button></>}>
            <window.Field label="Name" error={err}><window.Input mono autoFocus placeholder="ci-deploy" value={form.name} error={err} onChange={(e) => setForm({ ...form, name: e.target.value })} /></window.Field>
            <window.Field label="Description" hint="Optional"><window.Input placeholder="What is this identity for?" value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} /></window.Field>
            <window.Field label="Role"><window.Select value={form.role} onChange={(e) => setForm({ ...form, role: e.target.value })}><option value="viewer">Viewer</option><option value="admin">Admin</option><option value="owner">Owner</option></window.Select></window.Field>
          </window.Modal>
        )}
        {confirmDel && (
          <window.Modal title={`Delete ${confirmDel.name}?`} icon="trash" onClose={() => setConfirmDel(null)}
            desc="This invalidates all admin API keys associated with this service account."
            footer={<><window.Button variant="ghost" onClick={() => setConfirmDel(null)}>Cancel</window.Button><window.Button variant="danger" icon="trash" onClick={() => { deleteServiceAccount(confirmDel.id); setConfirmDel(null); toast({ msg: "Service account deleted", icon: "trash", tone: "red" }); }}>Delete</window.Button></>} />
        )}
      </div>
    );
  }

  function Identities() {
    const { members, serviceAccounts } = window.useApp();
    const [tab, setTab] = useState("members");
    return (
      <div className="page page-wide">
        <div className="page-head"><div><h1 className="page-title">Identities</h1><p className="page-sub">People and machines with access to this account</p></div></div>
        <window.Tabs value={tab} onChange={setTab} tabs={[
          { value: "members", label: "Members", icon: "users", count: members.length },
          { value: "service", label: "Service Accounts", icon: "bot", count: serviceAccounts.length },
        ]} />
        {tab === "members" ? <Members /> : <ServiceAccounts />}
      </div>
    );
  }
  return Identities;
})();
window.IdentitiesView = IdentitiesView;
