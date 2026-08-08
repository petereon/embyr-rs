/* embyr console — mock data layer (plain JS, global) */
(function () {
  "use strict";

  // ---- helpers ----------------------------------------------------------
  function rng(seed) {
    // deterministic pseudo-random so renders are stable
    let s = seed % 2147483647;
    if (s <= 0) s += 2147483646;
    return function () {
      s = (s * 16807) % 2147483647;
      return (s - 1) / 2147483646;
    };
  }
  const r = rng(42);
  function pick(arr) { return arr[Math.floor(r() * arr.length)]; }
  function int(min, max) { return Math.floor(r() * (max - min + 1)) + min; }

  function fmtNum(n) {
    if (n >= 1e9) return (n / 1e9).toFixed(1).replace(/\.0$/, "") + "B";
    if (n >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, "") + "M";
    if (n >= 1e3) return (n / 1e3).toFixed(1).replace(/\.0$/, "") + "K";
    return String(n);
  }

  // ---- accounts ---------------------------------------------------------
  const accounts = [
    { id: "acc_personal", name: "Personal", plan: "Pro", initial: "P" },
    { id: "acc_side", name: "weekend-labs", plan: "Free", initial: "w" },
  ];

  // ---- series generators ------------------------------------------------
  // 60 points (minutes) of latency around a baseline
  function latencySeries(base, jitter) {
    const out = [];
    for (let i = 0; i < 60; i++) {
      const wobble = Math.sin(i / 7) * jitter * 0.5 + (r() - 0.5) * jitter;
      const spike = r() > 0.94 ? jitter * (1.5 + r() * 2) : 0;
      out.push(Math.max(1, Math.round(base + wobble + spike)));
    }
    return out;
  }
  // 24 points (hours) of op volume
  function opSeries(base) {
    const out = [];
    for (let i = 0; i < 24; i++) {
      const dayCurve = Math.sin(((i - 6) / 24) * Math.PI * 2) * 0.4 + 1;
      out.push(Math.max(0, Math.round(base * dayCurve * (0.7 + r() * 0.6))));
    }
    return out;
  }

  // ---- databases (projects) --------------------------------------------
  const databases = [
    {
      id: "db_prod_api", name: "prod-api", status: "active",
      backendMode: "direct_pg", secretBackend: null,
      connectionUrl: "db.prod.internal:5432/embyr_prod",
      agentEndpoint: null, secretName: null,
      created: "2025-08-14", region: "us-east-1",
      p95: 11, p99: 28, p50: 4,
      reads: 1240000, writes: 342000, deletes: 12400,
      activeStreams: 318, subscribe: 214, listen: 104,
      logging: true, retention: 7,
      listenHours: 48.2, logStorageGB: 2.1,
      latency: latencySeries(11, 6), ops: opSeries(54000),
    },
    {
      id: "db_staging", name: "staging", status: "active",
      backendMode: "agent_mode", secretBackend: "aws_secretsmanager",
      connectionUrl: null,
      agentEndpoint: "agent.staging.embyr.dev:8443",
      secretName: "arn:aws:secretsmanager:us-east-1:4417:secret:embyr/staging-Xa9",
      created: "2025-09-02", region: "us-east-1",
      p95: 19, p99: 44, p50: 7,
      reads: 88000, writes: 41000, deletes: 3100,
      activeStreams: 22, subscribe: 14, listen: 8,
      logging: true, retention: 1,
      listenHours: 9.4, logStorageGB: 0.4,
      latency: latencySeries(19, 9), ops: opSeries(4200),
    },
    {
      id: "db_analytics", name: "analytics", status: "active",
      backendMode: "agent_mode", secretBackend: "gcp_secretmanager",
      connectionUrl: null,
      agentEndpoint: "agent.analytics.embyr.dev:8443",
      secretName: "projects/embyr/secrets/analytics-pg/versions/3",
      created: "2025-10-21", region: "europe-west1",
      p95: 33, p99: 71, p50: 12,
      reads: 612000, writes: 9800, deletes: 220,
      activeStreams: 47, subscribe: 47, listen: 0,
      logging: false, retention: 7,
      listenHours: 21.0, logStorageGB: 0,
      latency: latencySeries(33, 14), ops: opSeries(26000),
    },
    {
      id: "db_playground", name: "playground", status: "active",
      backendMode: "direct_pg", secretBackend: null,
      connectionUrl: "localhost:5432/playground",
      agentEndpoint: null, secretName: null,
      created: "2026-01-09", region: "us-east-1",
      p95: 6, p99: 14, p50: 3,
      reads: 4200, writes: 1800, deletes: 90,
      activeStreams: 1, subscribe: 1, listen: 0,
      logging: false, retention: 1,
      listenHours: 0.3, logStorageGB: 0,
      latency: latencySeries(6, 4), ops: opSeries(220),
    },
    {
      id: "db_legacy", name: "legacy-store", status: "suspended",
      backendMode: "direct_pg", secretBackend: null,
      connectionUrl: "db.legacy.internal:5432/store",
      agentEndpoint: null, secretName: null,
      created: "2025-06-30", region: "us-west-2",
      p95: 0, p99: 0, p50: 0,
      reads: 0, writes: 0, deletes: 0,
      activeStreams: 0, subscribe: 0, listen: 0,
      logging: false, retention: 30,
      listenHours: 0, logStorageGB: 0.8,
      latency: new Array(60).fill(0), ops: new Array(24).fill(0),
    },
  ];

  // ---- query logs -------------------------------------------------------
  const OPS = ["read", "write", "delete", "query", "listen", "subscribe"];
  const COLLECTIONS = [
    "users", "users/u_8821/sessions", "orders", "orders/o_4410/items",
    "telemetry/events", "products", "products/p_1190/reviews",
    "carts/c_3321", "audit_log", "feature_flags", "inventory/sku_9920",
    "messages/thread_77/replies", "devices/d_5512/state",
  ];
  const STATUSES = ["ok", "ok", "ok", "ok", "ok", "ok", "ok", "ok", "DEADLINE_EXCEEDED", "PERMISSION_DENIED", "NOT_FOUND"];
  function makeLogs(n) {
    const out = [];
    let t = Date.now();
    for (let i = 0; i < n; i++) {
      t -= int(200, 9000);
      const op = pick(OPS);
      const status = pick(STATUSES);
      out.push({
        id: "log_" + i,
        ts: t,
        op,
        path: pick(COLLECTIONS),
        duration: op === "subscribe" || op === "listen" ? int(1, 4) : int(2, 180),
        status,
        client: pick(["a7f3c1", "9b2e44", "c01d8a", "ff2901", "3e77bd"]),
      });
    }
    return out;
  }
  const queryLogs = { db_prod_api: makeLogs(240), db_staging: makeLogs(90) };

  // ---- identities -------------------------------------------------------
  const members = [
    { id: "u_owner", email: "you@personal.dev", name: "You", role: "owner", auth: "email_password", mfa: "TOTP", lastLogin: "2026-06-05T08:12:00Z", you: true },
    { id: "u_dana", email: "dana@contractor.io", name: "Dana Ruiz", role: "admin", auth: "oidc", mfa: "—", lastLogin: "2026-06-04T19:40:00Z" },
    { id: "u_kofi", email: "kofi@studio.cc", name: "Kofi Mensah", role: "viewer", auth: "email_password", mfa: "Email OTP", lastLogin: "2026-05-28T11:03:00Z" },
    { id: "u_invite", email: "sam@newhire.dev", name: "—", role: "viewer", auth: "—", mfa: "—", lastLogin: null, pending: true },
  ];

  const serviceAccounts = [
    { id: "sa_ci", name: "ci-deploy", description: "GitHub Actions deploy bot", role: "admin", created: "2025-09-10", lastUsed: "2026-06-05T06:30:00Z" },
    { id: "sa_backup", name: "nightly-backup", description: "Cron backup runner", role: "viewer", created: "2025-11-01", lastUsed: "2026-06-05T04:00:00Z" },
  ];

  // ---- api keys ---------------------------------------------------------
  const adminKeys = [
    { id: "ak_1", name: "ci-deploy key", identity: "ci-deploy", identityType: "service", role: "admin", created: "2025-09-10", lastUsed: "2026-06-05T06:30:00Z", prefix: "embyr_ad" },
    { id: "ak_2", name: "terraform", identity: "You", identityType: "member", role: "owner", created: "2025-08-15", lastUsed: "2026-06-02T14:11:00Z", prefix: "embyr_ad" },
    { id: "ak_3", name: "backup runner", identity: "nightly-backup", identityType: "service", role: "viewer", created: "2025-11-01", lastUsed: "2026-06-05T04:00:00Z", prefix: "embyr_ad" },
  ];

  const sdkKeys = {
    db_prod_api: [
      { id: "sk_1", name: "web-client", created: "2025-08-14", lastUsed: "2026-06-05T08:40:00Z", prefix: "embyr_sd" },
      { id: "sk_2", name: "ios-app", created: "2025-08-20", lastUsed: "2026-06-05T08:39:00Z", prefix: "embyr_sd" },
      { id: "sk_3", name: "edge-worker", created: "2026-02-11", lastUsed: "2026-06-05T08:41:00Z", prefix: "embyr_sd" },
    ],
    db_staging: [
      { id: "sk_4", name: "preview-deploys", created: "2025-09-02", lastUsed: "2026-06-04T22:10:00Z", prefix: "embyr_sd" },
    ],
    db_analytics: [
      { id: "sk_5", name: "etl-reader", created: "2025-10-21", lastUsed: "2026-06-05T07:55:00Z", prefix: "embyr_sd" },
    ],
    db_playground: [],
    db_legacy: [],
  };

  // ---- oidc providers ---------------------------------------------------
  const oidcProviders = [
    { id: "oidc_google", name: "Google", issuer: "https://accounts.google.com", clientId: "4417-prod.apps.googleusercontent.com", enabled: true },
    { id: "oidc_gh", name: "GitHub", issuer: "https://github.com/login/oauth", clientId: "Iv1.a1b2c3d4e5f6", enabled: false },
  ];

  // ---- expose -----------------------------------------------------------
  window.DATA = {
    accounts, databases, queryLogs, members, serviceAccounts,
    adminKeys, sdkKeys, oidcProviders,
    fmtNum,
    OPS, STATUSES,
    user: { name: "You", email: "you@personal.dev", role: "owner", initial: "Y" },
  };
})();
