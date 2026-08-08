/* embyr console — shell, router, tweaks, mount */
const { useState: useS, useEffect: useE } = React;
const Icon = window.Icon;

const ACCENT_PRESETS = {
  ember:   ["#e8632a", "#f4a35a", "#c44e0a"],
  amber:   ["#df9326", "#f3c45c", "#b16e0c"],
  crimson: ["#e2483f", "#f3796a", "#bb2a24"],
  emerald: ["#1faf6d", "#5fd49a", "#118a52"],
  violet:  ["#8b5cf6", "#b39bff", "#6d3fd6"],
};
const FONT_STACKS = {
  Geist: '"Geist", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
  Manrope: '"Manrope", -apple-system, BlinkMacSystemFont, sans-serif',
  System: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
};

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "look": "ember",
  "accent": ["#e8632a", "#f4a35a", "#c44e0a"],
  "density": "comfortable",
  "font": "Geist"
}/*EDITMODE-END*/;

/* ---------------- Sidebar ---------------- */
function Sidebar() {
  const { nav, go, account, accounts, setAccountId, databases } = window.useApp();
  const items = [
    { id: "dashboard", label: "Dashboard", icon: "dashboard" },
    { id: "databases", label: "Databases", icon: "database", count: databases.length },
    { id: "billing", label: "Billing", icon: "billing" },
    { id: "identities", label: "Identities", icon: "users" },
    { id: "apikeys", label: "API Keys", icon: "key" },
  ];
  return (
    <aside className="sidebar">
      <div className="brand">
        <img className="brand-mark" src="assets/embyr-mark.svg" alt="" />
        <span className="brand-word">embyr</span>
      </div>

      <window.Menu align="left" width={232} trigger={
        <button className="acct-switch">
          <span className="acct-badge">{account.initial}</span>
          <span className="acct-meta"><span className="acct-name">{account.name}</span><span className="acct-plan">{account.plan} plan</span></span>
          <Icon name="chevronsUpDown" size={15} style={{ color: "var(--text-3)", marginLeft: "auto" }} />
        </button>
      }>
        <div className="menu-label">Switch account</div>
        {accounts.map((a) => (
          <window.MenuItem key={a.id} icon={a.id === account.id ? "check" : undefined} onClick={() => { setAccountId(a.id); go("dashboard"); }}>
            <span style={{ paddingLeft: a.id === account.id ? 0 : 25 }}>{a.name}</span>
            <span className="spacer" /><span className="tag" style={{ height: 18 }}>{a.plan}</span>
          </window.MenuItem>
        ))}
        <div className="menu-sep" />
        <window.MenuItem icon="plus">Create account</window.MenuItem>
      </window.Menu>

      <nav className="nav">
        {items.map((it) => (
          <button key={it.id} className={`nav-item ${nav.section === it.id ? "active" : ""}`} onClick={() => go(it.id)}>
            <Icon name={it.icon} size={17} />{it.label}
            {it.count != null && <span className="nav-count">{it.count}</span>}
          </button>
        ))}
      </nav>

      <div className="sidebar-foot">
        <button className={`nav-item ${nav.section === "settings" ? "active" : ""}`} onClick={() => go("settings")}>
          <Icon name="settings" size={17} />Account Settings
        </button>
      </div>
    </aside>
  );
}

/* ---------------- Topbar ---------------- */
function Topbar({ onSignOut }) {
  const { nav, go, currentDb, user, account } = window.useApp();
  const SECTION_LABEL = { dashboard: "Dashboard", databases: "Databases", billing: "Billing", identities: "Identities", apikeys: "API Keys", settings: "Account Settings" };
  const SUB_LABEL = { overview: "Overview", connections: "Connections", logs: "Query Logs", keys: "API Keys" };

  return (
    <header className="topbar">
      <div className="crumbs">
        {nav.section === "databases" && currentDb ? (
          <>
            <span className="crumb-link" onClick={() => go("databases")}>Databases</span>
            <Icon name="chevronRight" size={14} />
            <span className="crumb-cur mono">{currentDb.name}</span>
            <Icon name="chevronRight" size={14} />
            <span className="crumb-cur">{SUB_LABEL[nav.sub] || "Overview"}</span>
          </>
        ) : (
          <span className="crumb-cur">{SECTION_LABEL[nav.section]}</span>
        )}
      </div>
      <div className="topbar-spacer" />
      <div className="row" style={{ gap: 4 }}>
        <button className="iconbtn" title="Docs"><Icon name="external" size={17} /></button>
        <window.Menu trigger={<button className="iconbtn" title="Notifications" style={{ position: "relative" }}><Icon name="bell" size={17} /><span style={{ position: "absolute", top: 7, right: 8, width: 6, height: 6, borderRadius: 99, background: "var(--accent)", border: "1.5px solid var(--bg)" }} /></button>} width={260}>
          <div className="menu-label">Notifications</div>
          <div style={{ padding: "8px 10px", fontSize: 12.5, color: "var(--text-2)", display: "flex", gap: 9 }}><Icon name="alert" size={14} style={{ color: "var(--amber)", marginTop: 1 }} /><span><b style={{ color: "var(--text)" }}>analytics</b> P95 above 30ms for 12m</span></div>
          <div style={{ padding: "8px 10px", fontSize: 12.5, color: "var(--text-2)", display: "flex", gap: 9 }}><Icon name="mail" size={14} style={{ color: "var(--text-3)", marginTop: 1 }} /><span>Invite to <b style={{ color: "var(--text)" }}>sam@newhire.dev</b> is pending</span></div>
        </window.Menu>
        <window.Menu trigger={<button className="iconbtn" style={{ width: "auto", padding: "0 5px 0 4px", gap: 7 }}><window.Avatar name={user.name} size={26} /><Icon name="chevronDown" size={14} style={{ color: "var(--text-3)" }} /></button>} width={210}>
          <div style={{ padding: "8px 10px 6px" }}><div style={{ fontWeight: 600, fontSize: 13 }}>{user.name}</div><div style={{ fontSize: 11.5, color: "var(--text-3)" }}>{user.email}</div></div>
          <div className="menu-sep" />
          <window.MenuItem icon="settings" onClick={() => go("settings")}>Account Settings</window.MenuItem>
          <window.MenuItem icon="shield" onClick={() => go("settings")}>Security</window.MenuItem>
          <div className="menu-sep" />
          <window.MenuItem icon="logout" danger onClick={onSignOut}>Sign out</window.MenuItem>
        </window.Menu>
      </div>
    </header>
  );
}

/* ---------------- Router ---------------- */
function Routed() {
  const { nav } = window.useApp();
  if (nav.section === "dashboard") return <window.DashView />;
  if (nav.section === "databases") return nav.dbId ? <window.DatabaseDetail /> : <window.DatabasesView />;
  if (nav.section === "billing") return <window.BillingView />;
  if (nav.section === "identities") return <window.IdentitiesView />;
  if (nav.section === "apikeys") return <window.ApiKeysView />;
  if (nav.section === "settings") return <window.SettingsView />;
  return <window.DashView />;
}

/* ---------------- Tweaks ---------------- */
function ConsoleTweaks() {
  const [t, setTweak] = window.useTweaks(TWEAK_DEFAULTS);
  useE(() => {
    const r = document.documentElement;
    r.setAttribute("data-look", t.look);
    r.setAttribute("data-density", t.density === "compact" ? "compact" : "comfortable");
    const [a, a2, ad] = Array.isArray(t.accent) ? t.accent : ACCENT_PRESETS.ember;
    r.style.setProperty("--accent", a);
    r.style.setProperty("--accent-2", a2);
    r.style.setProperty("--accent-deep", ad);
    r.style.setProperty("--font-sans", FONT_STACKS[t.font] || FONT_STACKS.Geist);
  }, [t]);

  return (
    <window.TweaksPanel>
      <window.TweakSection label="Visual look" />
      <window.TweakRadio label="Theme" value={t.look} options={["ember", "graphite", "paper"]} onChange={(v) => setTweak("look", v)} />
      <window.TweakColor label="Accent" value={t.accent}
        options={[ACCENT_PRESETS.ember, ACCENT_PRESETS.amber, ACCENT_PRESETS.crimson, ACCENT_PRESETS.emerald, ACCENT_PRESETS.violet]}
        onChange={(v) => setTweak("accent", v)} />
      <window.TweakSection label="Layout & type" />
      <window.TweakRadio label="Density" value={t.density} options={["comfortable", "compact"]} onChange={(v) => setTweak("density", v)} />
      <window.TweakRadio label="Font" value={t.font} options={["Geist", "Manrope", "System"]} onChange={(v) => setTweak("font", v)} />
    </window.TweaksPanel>
  );
}

/* ---------------- Root ---------------- */
function Root() {
  const [authed, setAuthed] = useS(false);
  return (
    <window.ToastProvider>
      {!authed ? (
        <window.AuthGate onAuthed={() => setAuthed(true)} />
      ) : (
        <window.AppProvider>
          <div className="app">
            <Sidebar />
            <div className="main">
              <Topbar onSignOut={() => setAuthed(false)} />
              <div className="content"><Routed /></div>
            </div>
          </div>
          <ConsoleTweaks />
        </window.AppProvider>
      )}
    </window.ToastProvider>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<Root />);
