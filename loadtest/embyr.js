// k6 load test for embyr-server's Firestore data plane over gRPC-Web on :8081.
//
// Why hand-rolled protobuf: :8081 serves Firestore as gRPC-Web only (content-type
// application/grpc-web+proto, binary protobuf). There is no JSON/REST documents API, so no SDK
// is needed but the request bodies must be protobuf. The few messages used are encoded below
// (field numbers from proto/google/firestore/v1/*.proto). Run `k6 run loadtest/embyr.js`.
// Everything is configured by env vars; see loadtest/README.md.
//
// Not covered: Listen/Write streams (long-lived HTTP/1.1 streaming is not a good fit for k6).

import http from 'k6/http';
import { check } from 'k6';
import { Counter, Rate } from 'k6/metrics';

// ---- config -----------------------------------------------------------------------------
const BASE_URL = (__ENV.BASE_URL || 'http://localhost:8081').replace(/\/$/, '');
const ADMIN_URL = (__ENV.ADMIN_URL || 'http://localhost:9090').replace(/\/$/, '');
const PROJECT_ID = __ENV.PROJECT_ID || 'loadtest';
const API_KEY = __ENV.API_KEY || '';
const ADMIN_KEY = __ENV.ADMIN_KEY || ''; // only to provision a project in setup()
const CUSTOMER_DSN = __ENV.CUSTOMER_DSN || ''; // only to provision a project in setup()
const PROFILE = __ENV.PROFILE || 'smoke'; // smoke | ramp | soak
const SCENARIOS = (__ENV.SCENARIOS || 'data').split(','); // data, badkeys
const TARGET_RPS = Number(__ENV.TARGET_RPS || 200); // total across the data scenarios
const SOAK_DURATION = __ENV.SOAK_DURATION || '30m';
const SEED_DOCS = Number(__ENV.SEED_DOCS || 500);
const WRITE_KEYSPACE = Number(__ENV.WRITE_KEYSPACE || 10000);
const PAYLOAD_BYTES = Number(__ENV.PAYLOAD_BYTES || 256);
const COLLECTION = __ENV.COLLECTION || 'loadtest';
const QUERY_LIMIT = Number(__ENV.QUERY_LIMIT || 20);
const BAD_KEY_RPS = Number(__ENV.BAD_KEY_RPS || 20);
const BAD_KEY_DURATION_S = Number(__ENV.BAD_KEY_DURATION_S || 120);
const REPLICAS = Number(__ENV.REPLICAS || 1); // replicas behind BASE_URL, scales the Argon2 bound

// Mix of the data scenarios (sums to 1). "update" is UpdateDocument (the REST PATCH equivalent).
const MIX = { get: 0.6, commit: 0.2, update: 0.1, query: 0.1 };

// Thresholds mirror docs/operations/slo.md (client-side, so they include network time: run k6
// close to the server). Override with env while calibrating.
const P99_GET_MS = Number(__ENV.P99_GET_MS || 250);
const P99_COMMIT_MS = Number(__ENV.P99_COMMIT_MS || 500);
const P99_QUERY_MS = Number(__ENV.P99_QUERY_MS || 500);
const MAX_ERROR_RATE = Number(__ENV.MAX_ERROR_RATE || 0.001); // SLO 99.9 %

// ---- metrics ----------------------------------------------------------------------------
// "Server fault" = same set as docs/operations/slo.md SLI 1 (or a non-200 from the LB).
const serverFault = new Rate('grpc_server_fault');
const rateLimited = new Counter('grpc_rate_limited'); // resource_exhausted on data scenarios
const badKeyUnauth = new Counter('badkey_unauthenticated'); // an Argon2 verification ran
const badKeyShed = new Counter('badkey_resource_exhausted'); // refused before Argon2
const badKeyOther = new Counter('badkey_unexpected');
const legitOk = new Rate('legit_during_flood_ok');

// ---- options ----------------------------------------------------------------------------
function stages(rate) {
  const r = Math.max(1, Math.round(rate));
  if (PROFILE === 'ramp') {
    return [
      { duration: '1m', target: Math.max(1, Math.round(r * 0.25)) },
      { duration: '2m', target: Math.max(1, Math.round(r * 0.5)) },
      { duration: '2m', target: Math.max(1, Math.round(r * 0.75)) },
      { duration: '2m', target: r },
      { duration: '5m', target: r }, // hold
      { duration: '30s', target: 0 },
    ];
  }
  if (PROFILE === 'soak') {
    return [
      { duration: '2m', target: r },
      { duration: SOAK_DURATION, target: r },
      { duration: '1m', target: 0 },
    ];
  }
  const s = Math.max(1, Math.round(Math.min(TARGET_RPS, 20) * (rate / TARGET_RPS)));
  return [{ duration: '10s', target: s }, { duration: '30s', target: s }];
}

function arrival(execName, rate) {
  const r = Math.max(1, Math.round(rate));
  return {
    executor: 'ramping-arrival-rate',
    startRate: 1,
    timeUnit: '1s',
    preAllocatedVUs: Math.max(10, Math.ceil(r * 0.2)),
    maxVUs: Math.max(50, r * 2),
    stages: stages(rate),
    exec: execName,
  };
}

const scenarios = {};
const thresholds = {};
if (SCENARIOS.includes('data')) {
  scenarios.get = arrival('getDocument', TARGET_RPS * MIX.get);
  scenarios.commit = arrival('commit', TARGET_RPS * MIX.commit);
  scenarios.update = arrival('updateDocument', TARGET_RPS * MIX.update);
  scenarios.query = arrival('runQuery', TARGET_RPS * MIX.query);
  thresholds['grpc_server_fault'] = [`rate<${MAX_ERROR_RATE}`];
  thresholds['http_req_duration{rpc:GetDocument}'] = [`p(99)<${P99_GET_MS}`];
  thresholds['http_req_duration{rpc:Commit}'] = [`p(99)<${P99_COMMIT_MS}`];
  thresholds['http_req_duration{rpc:UpdateDocument}'] = [`p(99)<${P99_COMMIT_MS}`];
  thresholds['http_req_duration{rpc:RunQuery}'] = [`p(99)<${P99_QUERY_MS}`];
  // The per-project limiter (EMBYR_RATE_LIMIT_RPS, default 1000) would otherwise be measured
  // instead of the server. Raise it on the server under test; this makes the run fail loudly.
  thresholds['grpc_rate_limited'] = ['count==0'];
}
if (SCENARIOS.includes('badkeys')) {
  // Many DISTINCT wrong keys for a REAL project id. Expected (credential_cache.rs): per project,
  // per process, 10 attempts burst + 1/s refill reach Argon2 (-> unauthenticated); the rest are
  // shed with resource_exhausted before any Argon2 work. Bound is scaled by REPLICAS, +20 % slack.
  const bound = Math.ceil(REPLICAS * (10 + BAD_KEY_DURATION_S) * 1.2) + 10;
  scenarios.badkeys = {
    executor: 'constant-arrival-rate',
    rate: BAD_KEY_RPS,
    timeUnit: '1s',
    duration: `${BAD_KEY_DURATION_S}s`,
    preAllocatedVUs: Math.max(10, BAD_KEY_RPS),
    maxVUs: Math.max(50, BAD_KEY_RPS * 4),
    exec: 'badKey',
  };
  // Meanwhile a legitimate, already-verified key must keep working.
  scenarios.legit_during_flood = {
    executor: 'constant-arrival-rate',
    rate: 5,
    timeUnit: '1s',
    duration: `${BAD_KEY_DURATION_S}s`,
    preAllocatedVUs: 5,
    maxVUs: 20,
    exec: 'legitDuringFlood',
  };
  thresholds['badkey_unauthenticated'] = [`count<=${bound}`];
  thresholds['badkey_resource_exhausted'] = ['count>0']; // the gate engaged at all
  thresholds['badkey_unexpected'] = ['count==0'];
  thresholds['legit_during_flood_ok'] = ['rate>0.99'];
  thresholds['http_req_duration{rpc:LegitDuringFlood}'] = [`p(99)<${P99_GET_MS}`];
}

export const options = { scenarios, thresholds, discardResponseBodies: false };

// ---- minimal protobuf encoder (ASCII strings only) -------------------------------------
function varint(n) {
  const out = [];
  while (n >= 128) {
    out.push((n % 128) | 128);
    n = Math.floor(n / 128);
  }
  out.push(n);
  return out;
}
const tag = (field, wire) => varint(field * 8 + wire);
const ascii = (s) => Array.from(s, (c) => c.charCodeAt(0) & 0x7f);
const msg = (field, bytes) => tag(field, 2).concat(varint(bytes.length), bytes);
const str = (field, s) => msg(field, ascii(s));
const int = (field, n) => tag(field, 0).concat(varint(n));

// Value { string_value = 17, integer_value = 2 }
const valueStr = (s) => str(17, s);
const valueInt = (n) => int(2, n);
// Document { name = 1, fields = 2 (map<string, Value> => repeated entry{key=1, value=2}) }
function document(name, fields) {
  let b = str(1, name);
  for (const k of Object.keys(fields)) {
    const v = fields[k];
    const val = typeof v === 'number' ? valueInt(v) : valueStr(v);
    b = b.concat(msg(2, str(1, k).concat(msg(2, val))));
  }
  return b;
}

const DB = `projects/${PROJECT_ID}/databases/(default)`;
const PARENT = `${DB}/documents`;
const docName = (id) => `${PARENT}/${COLLECTION}/${id}`;
const payload = 'x'.repeat(PAYLOAD_BYTES);

// GetDocumentRequest { name = 1 }
const getReq = (id) => str(1, docName(id));
// CommitRequest { database = 1, writes = 2 (Write { update = 1 }) }
const commitReq = (id, n) =>
  str(1, DB).concat(msg(2, msg(1, document(docName(id), { n: n, counter: 1, payload: payload }))));
// UpdateDocumentRequest { document = 1, update_mask = 2 (DocumentMask { field_paths = 1 }) }
const updateReq = (id) =>
  msg(1, document(docName(id), { counter: Math.floor(Math.random() * 1e6), payload: payload })).concat(
    msg(2, str(1, 'counter').concat(str(1, 'payload'))),
  );
// RunQueryRequest { parent = 1, structured_query = 2 }
// StructuredQuery { from = 2 (CollectionSelector { collection_id = 2 }), order_by = 4
//   (Order { field = 1 (FieldReference { field_path = 2 }), direction = 2 }), limit = 5 (Int32Value { value = 1 }) }
const queryReq = () =>
  str(1, PARENT).concat(
    msg(
      2,
      msg(2, str(2, COLLECTION)).concat(msg(4, msg(1, str(2, 'n')).concat(int(2, 1))), msg(5, int(1, QUERY_LIMIT))),
    ),
  );

// gRPC-Web data frame: 1 flag byte (0) + 4 byte big-endian length + message.
function frame(bytes) {
  const n = bytes.length;
  return new Uint8Array([0, (n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255].concat(bytes)).buffer;
}

// ---- gRPC-Web call + response parsing ---------------------------------------------------
const FAULT_CODES = { 2: 1, 4: 1, 13: 1, 14: 1, 15: 1 }; // unknown, deadline, internal, unavailable, data_loss
const RESOURCE_EXHAUSTED = 8;
const UNAUTHENTICATED = 16;

function header(res, name) {
  for (const k in res.headers) if (k.toLowerCase() === name) return res.headers[k];
  return undefined;
}

// Returns the grpc-status: from the trailer frame (flag bit 0x80) or, for trailers-only
// responses, from the HTTP headers. -1 = transport failure (non-200), -2 = no status found.
function grpcStatus(res) {
  if (res.status !== 200) return -1;
  const hs = header(res, 'grpc-status');
  if (hs !== undefined) return Number(hs);
  if (!res.body) return -2;
  const u8 = new Uint8Array(res.body);
  let i = 0;
  while (i + 5 <= u8.length) {
    const len = u8[i + 1] * 16777216 + u8[i + 2] * 65536 + u8[i + 3] * 256 + u8[i + 4];
    if (u8[i] & 0x80) {
      let text = '';
      for (let j = i + 5; j < i + 5 + len && j < u8.length; j++) text += String.fromCharCode(u8[j]);
      const m = /grpc-status:\s*(\d+)/i.exec(text);
      return m ? Number(m[1]) : -2;
    }
    i += 5 + len;
  }
  return -2;
}

function call(rpc, method, body, key) {
  const res = http.post(`${BASE_URL}/google.firestore.v1.Firestore/${method}`, frame(body), {
    headers: {
      'content-type': 'application/grpc-web+proto',
      accept: 'application/grpc-web+proto',
      'x-grpc-web': '1',
      authorization: `Bearer ${key}`,
    },
    responseType: 'binary',
    tags: { rpc: rpc },
  });
  return { res: res, status: grpcStatus(res) };
}

function dataCall(rpc, method, body, key) {
  const { res, status } = call(rpc, method, body, key);
  if (status === RESOURCE_EXHAUSTED) rateLimited.add(1);
  serverFault.add(status === -1 || status === -2 || FAULT_CODES[status] === 1, { rpc: rpc });
  check(res, { [`${rpc} grpc-status OK`]: () => status === 0 });
}

const rid = (n) => `d-${Math.floor(Math.random() * n)}`;

// ---- scenarios --------------------------------------------------------------------------
export function getDocument(d) {
  dataCall('GetDocument', 'GetDocument', getReq(rid(d.seedDocs)), d.apiKey);
}
export function commit(d) {
  dataCall('Commit', 'Commit', commitReq(`w-${Math.floor(Math.random() * WRITE_KEYSPACE)}`, Date.now() % 1e9), d.apiKey);
}
export function updateDocument(d) {
  dataCall('UpdateDocument', 'UpdateDocument', updateReq(rid(d.seedDocs)), d.apiKey);
}
export function runQuery(d) {
  dataCall('RunQuery', 'RunQuery', queryReq(), d.apiKey);
}

let badKeySeq = 0;
export function badKey() {
  // Unique per iteration so each one is a cache miss ("never verified" key).
  const key = `wrong-${__VU}-${__ITER}-${badKeySeq++}-${Math.random().toString(36).slice(2)}`;
  const { status } = call('BadKey', 'GetDocument', getReq('d-0'), key);
  if (status === UNAUTHENTICATED) badKeyUnauth.add(1);
  else if (status === RESOURCE_EXHAUSTED) badKeyShed.add(1);
  else badKeyOther.add(1);
}
export function legitDuringFlood(d) {
  const { status } = call('LegitDuringFlood', 'GetDocument', getReq(rid(d.seedDocs)), d.apiKey);
  legitOk.add(status === 0);
}

// ---- seed -------------------------------------------------------------------------------
export function setup() {
  let apiKey = API_KEY;
  if (!apiKey) {
    if (!ADMIN_KEY || !CUSTOMER_DSN) {
      throw new Error('set API_KEY, or ADMIN_KEY + CUSTOMER_DSN so setup() can provision PROJECT_ID');
    }
    const r = http.post(
      `${ADMIN_URL}/admin/v1/projects`,
      JSON.stringify({ project_id: PROJECT_ID, dsn: CUSTOMER_DSN }),
      { headers: { 'content-type': 'application/json', authorization: `Bearer ${ADMIN_KEY}` } },
    );
    if (r.status < 200 || r.status > 299) {
      throw new Error(`provision ${PROJECT_ID} failed: HTTP ${r.status} ${r.body} (409 = already exists: pass API_KEY)`);
    }
    apiKey = r.json('api_key');
    // The key is only returned once; print it on request so later runs can pass API_KEY.
    if (__ENV.PRINT_API_KEY) console.log(`provisioned ${PROJECT_ID}, api_key=${apiKey}`);
  }

  // Seed docs d-0..d-(N-1) with an integer `n` (orderBy target) via batched Commits.
  const BATCH = 50;
  for (let start = 0; start < SEED_DOCS; start += BATCH) {
    let writes = [];
    for (let i = start; i < Math.min(SEED_DOCS, start + BATCH); i++) {
      writes = writes.concat(msg(2, msg(1, document(docName(`d-${i}`), { n: i, counter: 0, payload: payload }))));
    }
    const { res, status } = call('Seed', 'Commit', str(1, DB).concat(writes), apiKey);
    if (status !== 0) throw new Error(`seed Commit failed: HTTP ${res.status} grpc-status ${status}`);
  }
  return { apiKey: apiKey, seedDocs: SEED_DOCS };
}
