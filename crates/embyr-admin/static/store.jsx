/* embyr console — app store (nav + mutable data). Exports useApp + AppProvider. */
const { useState: useSt, useCallback: useCb, useContext: useCtx, createContext: createCtx } = React;

const AppContext = createCtx(null);
function useApp() { return useCtx(AppContext); }

function genKey(prefix) {
  const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  let s = "";
  for (let i = 0; i < 32; i++) s += chars[Math.floor(Math.random() * chars.length)];
  return `${prefix}_${s}`;
}

function AppProvider({ children }) {
  const D = window.DATA;
  const [nav, setNav] = useSt({ section: "dashboard", dbId: null, sub: "overview" });
  const [accountId, setAccountId] = useSt(D.accounts[0].id);

  const [databases, setDatabases] = useSt(D.databases);
  const [members, setMembers] = useSt(D.members);
  const [serviceAccounts, setServiceAccounts] = useSt(D.serviceAccounts);
  const [adminKeys, setAdminKeys] = useSt(D.adminKeys);
  const [sdkKeys, setSdkKeys] = useSt(D.sdkKeys);
  const [oidcProviders, setOidc] = useSt(D.oidcProviders);

  /* ---- navigation ---- */
  const go = useCb((section, opts = {}) => {
    setNav({ section, dbId: opts.dbId ?? null, sub: opts.sub ?? "overview" });
    const c = document.querySelector(".content");
    if (c) c.scrollTop = 0;
  }, []);
  const openDb = useCb((dbId, sub = "overview") => { go("databases", { dbId, sub }); }, [go]);

  const currentDb = databases.find((d) => d.id === nav.dbId) || null;
  const account = D.accounts.find((a) => a.id === accountId);

  /* ---- mutations ---- */
  const createDatabase = useCb((d) => {
    const id = "db_" + d.name.replace(/[^a-z0-9]/gi, "_").toLowerCase() + "_" + Math.random().toString(36).slice(2, 6);
    const fresh = {
      id, name: d.name, status: "active", backendMode: d.backendMode,
      secretBackend: d.secretBackend || null, connectionUrl: d.connectionUrl || null,
      agentEndpoint: d.agentEndpoint || null, secretName: d.secretName || null,
      created: new Date().toISOString().slice(0, 10), region: d.region || "us-east-1",
      p95: 0, p99: 0, p50: 0, reads: 0, writes: 0, deletes: 0,
      activeStreams: 0, subscribe: 0, listen: 0, logging: false, retention: 7,
      listenHours: 0, logStorageGB: 0, latency: new Array(60).fill(0), ops: new Array(24).fill(0),
    };
    setDatabases((arr) => [fresh, ...arr]);
    setSdkKeys((m) => ({ ...m, [id]: [] }));
    return fresh;
  }, []);
  const setDbStatus = useCb((id, status) => setDatabases((arr) => arr.map((d) => d.id === id ? { ...d, status } : d)), []);
  const deleteDatabase = useCb((id) => {
    setDatabases((arr) => arr.filter((d) => d.id !== id));
  }, []);
  const patchDb = useCb((id, patch) => setDatabases((arr) => arr.map((d) => d.id === id ? { ...d, ...patch } : d)), []);

  const addSdkKey = useCb((dbId, name) => {
    const full = genKey("embyr_sdk");
    const k = { id: "sk_" + Math.random().toString(36).slice(2, 7), name, created: new Date().toISOString().slice(0, 10), lastUsed: null, prefix: full.slice(0, 8) };
    setSdkKeys((m) => ({ ...m, [dbId]: [k, ...(m[dbId] || [])] }));
    return { ...k, full };
  }, []);
  const revokeSdkKey = useCb((dbId, keyId) => setSdkKeys((m) => ({ ...m, [dbId]: (m[dbId] || []).filter((k) => k.id !== keyId) })), []);

  const addAdminKey = useCb((d) => {
    const full = genKey("embyr_adm");
    const k = { id: "ak_" + Math.random().toString(36).slice(2, 7), name: d.name, identity: d.identity, identityType: d.identityType, role: d.role, created: new Date().toISOString().slice(0, 10), lastUsed: null, prefix: full.slice(0, 8) };
    setAdminKeys((arr) => [k, ...arr]);
    return { ...k, full };
  }, []);
  const revokeAdminKey = useCb((id) => setAdminKeys((arr) => arr.filter((k) => k.id !== id)), []);

  const inviteMember = useCb((email, role) => {
    const m = { id: "u_" + Math.random().toString(36).slice(2, 7), email, name: "—", role, auth: "—", mfa: "—", lastLogin: null, pending: true };
    setMembers((arr) => [...arr, m]);
  }, []);
  const setMemberRole = useCb((id, role) => setMembers((arr) => arr.map((m) => m.id === id ? { ...m, role } : m)), []);
  const removeMember = useCb((id) => setMembers((arr) => arr.filter((m) => m.id !== id)), []);

  const createServiceAccount = useCb((d) => {
    const sa = { id: "sa_" + Math.random().toString(36).slice(2, 7), name: d.name, description: d.description || "—", role: d.role, created: new Date().toISOString().slice(0, 10), lastUsed: null };
    setServiceAccounts((arr) => [...arr, sa]);
  }, []);
  const deleteServiceAccount = useCb((id) => setServiceAccounts((arr) => arr.filter((s) => s.id !== id)), []);

  const toggleOidc = useCb((id) => setOidc((arr) => arr.map((p) => p.id === id ? { ...p, enabled: !p.enabled } : p)), []);

  const value = {
    nav, go, openDb, accountId, setAccountId, account, accounts: D.accounts,
    databases, currentDb, createDatabase, setDbStatus, deleteDatabase, patchDb,
    members, inviteMember, setMemberRole, removeMember,
    serviceAccounts, createServiceAccount, deleteServiceAccount,
    adminKeys, addAdminKey, revokeAdminKey,
    sdkKeys, addSdkKey, revokeSdkKey,
    oidcProviders, toggleOidc,
    user: D.user, fmtNum: D.fmtNum,
  };
  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

window.useApp = useApp;
window.AppProvider = AppProvider;
window.genKey = genKey;
