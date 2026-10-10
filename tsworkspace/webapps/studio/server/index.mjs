// JSON bridge in front of the trogon-atlas gRPC service. The browser cannot
// speak tonic gRPC directly, so this thin server is the only Node-side
// component: it loads the canonical proto and exposes read endpoints.

/** @typedef {import('express').Request & { requestId?: string }} AppRequest */
/** @typedef {{ status?: number } & Error} HttpError */
/** @typedef {import('express').Response} Res */
/** @typedef {import('express').NextFunction} Next */

import { AsyncLocalStorage } from "node:async_hooks";
import { createHash, timingSafeEqual } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import grpc from "@grpc/grpc-js";
import protoLoader from "@grpc/proto-loader";
import express from "express";
import protobuf from "protobufjs";
import { isSafeBranchName, isSafeNamespace } from "../shared/safe-namespace.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// The studio reads the canonical proto tree directly. We load `service.proto`
// (not `event_model.proto`) because it imports the entry plus every
// part-file the service needs; loading the entry alone would not pull
// in the service definitions, since the entry deliberately does not
// import service.proto (that would be a circular import: service.proto
// already imports event_model.proto for the Entity union).
const PROTO_PATH =
  process.env.PROTO_PATH ??
  path.resolve(__dirname, "../../../../proto/trogonatlas/api/eventmodel/v1alpha1/service.proto");
const PROTO_INCLUDES = [path.resolve(__dirname, "../../../../proto")];

const GRPC_ENDPOINT = process.env.TROGON_ATLAS_GRPC ?? "http://127.0.0.1:50069";
const AUTH_TOKEN = process.env.TROGON_ATLAS_AUTH_TOKEN ?? "";
// Whose credential reaches the gRPC server.
//
// Off (the default), the bridge holds one token and every upstream call is
// made as that one principal, whoever the browser is. That is fine for a
// single-operator deployment and wrong for a shared one: the server scopes
// namespaces per principal, and a bridge that substitutes its own identity
// hands every caller the union of what the bridge can see.
//
// On, the caller's own bearer token is forwarded and the server decides what
// that principal may read and write. The bridge stops being an authority: it
// cannot validate a token it did not issue, so it checks only that one was
// presented and lets the 401 come from upstream.
const AUTH_PASSTHROUGH = (process.env.TROGON_ATLAS_AUTH_PASSTHROUGH ?? "") === "true";
const DEFAULT_PORT = 8787;
const _parsedPort = Number(process.env.PORT);
const PORT = Number.isInteger(_parsedPort) && _parsedPort > 0 ? _parsedPort : DEFAULT_PORT;
const HOST = process.env.HOST ?? "127.0.0.1";
// Hard ceilings: keep a misconfigured or adversarial upstream from
// turning a single HTTP request into an unbounded server-side allocation.
const DEFAULT_MAX_PAGES = 50;
const _parsedMaxPages = Number(process.env.TROGON_ATLAS_STUDIO_MAX_PAGES ?? DEFAULT_MAX_PAGES);
const MAX_PAGES =
  Number.isInteger(_parsedMaxPages) && _parsedMaxPages > 0
    ? _parsedMaxPages
    : DEFAULT_MAX_PAGES;
const DEFAULT_GRPC_DEADLINE_MS = 15_000;
const _parsedGrpcDeadline = Number(
  process.env.TROGON_ATLAS_STUDIO_GRPC_TIMEOUT_MS ?? DEFAULT_GRPC_DEADLINE_MS,
);
const GRPC_DEADLINE_MS =
  Number.isInteger(_parsedGrpcDeadline) && _parsedGrpcDeadline > 0
    ? _parsedGrpcDeadline
    : DEFAULT_GRPC_DEADLINE_MS;
const RATE_LIMIT_WINDOW_MS = Number(
  process.env.TROGON_ATLAS_STUDIO_RATE_WINDOW_MS ?? 60_000,
);
const RATE_LIMIT_MAX = Number(
  process.env.TROGON_ATLAS_STUDIO_RATE_MAX ?? 600,
);
// Comma-separated allowlist of browser origins permitted to call /api/*.
// Empty (default) keeps the same-origin posture: no CORS headers emitted,
// no cross-origin preflight permitted. Set to "https://studio.example.com"
// (or "*" for fully public read APIs) to opt in.
const CORS_ORIGINS = (process.env.TROGON_ATLAS_STUDIO_CORS_ORIGINS ?? "")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
const NATS_WS_AUTH_TOKEN = process.env.NATS_WS_AUTH_TOKEN ?? "";
// Used to restrict the CSP connect-src directive to the configured NATS WS
// origin rather than allowing bare `ws:` / `wss:` scheme wildcards.
// Accepts a full WS URL, e.g. "ws://nats.example.com:8080".
// Falls back to 'self' only when unset (same-origin deployments where NATS is
// proxied through the gateway itself do not need an extra origin entry).
const NATS_WS_URL = process.env.TROGON_ATLAS_STUDIO_NATS_WS_URL ?? "";
const NAMESPACE_QUERY_MAX_LENGTH = 4096;

// Derive a safe WS origin token for the CSP connect-src directive from the
// configured NATS WS URL. Returns an empty string when the URL is absent or
// unparseable (the CSP then falls back to 'self' only).
function natsWsOriginForCsp() {
  if (!NATS_WS_URL) return "";
  try {
    const u = new URL(NATS_WS_URL);
    // Only ws: and wss: schemes are valid for a NATS WS endpoint. Reject
    // anything else to avoid letting an operator accidentally inject a rogue
    // origin via misconfiguration.
    if (u.protocol !== "ws:" && u.protocol !== "wss:") return "";
    return u.origin; // e.g. "ws://nats.example.com:8080"
  } catch {
    return "";
  }
}

const NATS_WS_CSP_ORIGIN = natsWsOriginForCsp();

// `GRPC_ENDPOINT` can carry credentials in the URL (e.g. user:pass@host).
// Strip those before any log line so we never leak them via journald/stdout.
/** @param {string} endpoint */
function maskEndpoint(endpoint) {
  try {
    const u = new URL(endpoint);
    if (u.username || u.password) {
      return `${u.protocol}//***@${u.host}${u.pathname}`;
    }
    return endpoint;
  } catch {
    return endpoint;
  }
}

/** @param {unknown} err */
function maskError(err) {
  // Strip anything that looks like a bearer header or basic-auth string
  // from outbound log lines. gRPC error messages can echo metadata back.
  const text = String(/** @type {any} */ (err)?.details ?? /** @type {any} */ (err)?.message ?? err ?? "");
  return text
    .replace(/(authorization:\s*Bearer\s+)\S+/gi, "$1***")
    .replace(/\/\/[^@\s]+@/g, "//***@");
}

/** @param {string} level @param {string} msg @param {Record<string, unknown>} [extra] */
function logEvent(level, msg, extra = {}) {
  const line = JSON.stringify({
    ts: new Date().toISOString(),
    level,
    msg,
    ...extra,
  });
  if (level === "error") process.stderr.write(line + "\n");
  else process.stdout.write(line + "\n");
}

// Warn at startup when the server is bound to a non-loopback address without
// an auth token configured. Every /api/* route including POST /api/delete-by-query
// is exposed to the network unauthenticated in this configuration.
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "::1", "localhost"]);
if (!LOOPBACK_HOSTS.has(HOST) && !AUTH_TOKEN && !AUTH_PASSTHROUGH) {
  logEvent("warn", "insecure_network_binding", {
    msg: "Server is bound to a non-loopback address without TROGON_ATLAS_AUTH_TOKEN -- every read under /api/* is exposed unauthenticated",
    host: HOST,
    port: PORT,
  });
}

// Warn at startup on the anonymous NATS WebSocket posture. The client-side
// warning in realtime.ts is stripped from production bundles (vite drops
// console.*), so this server-side line is the only signal operators get.
// Only when this bridge would actually send browsers there: one holding a
// credential answers `sse`, so the WebSocket posture is not its problem.
if (!LOOPBACK_HOSTS.has(HOST) && !NATS_WS_AUTH_TOKEN && !AUTH_TOKEN && !AUTH_PASSTHROUGH) {
  logEvent("warn", "anonymous_nats_websocket", {
    msg: "NATS_WS_AUTH_TOKEN is not set -- browsers connect to the NATS WebSocket unauthenticated. Safe for loopback deployments only",
    host: HOST,
    port: PORT,
  });
}

// There is no configuration in which the WebSocket token reaches a browser.
// One shared token is one shared permission set, which is not a per-principal
// answer no matter who holds it, so a bridge that authenticates callers sends
// them to the per-caller stream instead; and a bridge that does not
// authenticate callers cannot publish a credential to whoever asks. An
// operator who provisioned one should hear that it only forces the stream
// rather than assume the browsers are using it.
if (NATS_WS_AUTH_TOKEN) {
  logEvent("warn", "nats_token_unused", {
    msg: "NATS_WS_AUTH_TOKEN is set but is never handed to a browser -- it only tells /api/nats-auth to route the realtime feed through /api/changes/stream",
  });
}

// A bridge token in passthrough mode is not the identity anywhere except
// `/api/info`, the one route that runs before the auth guard can capture a
// caller's token. Say so once at boot: an operator who sets both and expects
// the bridge token to stand in more broadly would otherwise find out from an
// audit log.
if (AUTH_PASSTHROUGH) {
  logEvent("info", "auth_passthrough_enabled", {
    msg: AUTH_TOKEN
      ? "Forwarding each caller's own bearer token upstream; TROGON_ATLAS_AUTH_TOKEN is used only by /api/info, which runs before the auth guard"
      : "Forwarding each caller's own bearer token upstream; the bridge has no identity of its own",
  });
}

let definition;
try {
  definition = protoLoader.loadSync(PROTO_PATH, {
    keepCase: false,
    longs: String,
    enums: String,
    defaults: false,
    // No virtual oneof discriminators: they leak encoder internals into raw
    // JSON views (e.g. "kind":"string" inside FieldType). The studio derives
    // the envelope kind from whichever member key is present.
    oneofs: false,
    // include path so cross-file imports (e.g. "trogonatlas/type/v1alpha1/id.proto") resolve.
    includeDirs: PROTO_INCLUDES,
  });
} catch (err) {
  logEvent("error", "proto_loader_load_failed", {
    path: PROTO_PATH,
    error: maskError(err),
  });
  process.exit(1);
}
/** @type {{ EventModelService: new (target: string, credentials: grpc.ChannelCredentials) => Record<string, Function> }} */
const pkg = /** @type {any} */ (grpc.loadPackageDefinition(definition)).trogonatlas.api.eventmodel.v1alpha1;
const useTls = GRPC_ENDPOINT.startsWith("https://");
const target = GRPC_ENDPOINT.replace(/^https?:\/\//, "");
const credentials = useTls
  ? grpc.credentials.createSsl()
  : grpc.credentials.createInsecure();
const client = new pkg.EventModelService(target, credentials);

/**
 * @param {string | undefined} requestId
 * @param {string | undefined} [branch]
 * @param {{ allowBridgeIdentity?: boolean }} [opts]
 */
function metadata(requestId, branch, { allowBridgeIdentity = false } = {}) {
  const md = new grpc.Metadata();
  // In passthrough mode the caller's token is the identity. The bridge's own
  // token stands in only for the one route that runs before the auth guard
  // can set a caller's token (`/api/info`, via `allowBridgeIdentity`); any
  // other route reaching here with no forwarded token is a bug, not a caller
  // the bridge should quietly vouch for, so it fails closed instead of
  // substituting its own identity for whoever is actually asking.
  let token = AUTH_TOKEN;
  if (AUTH_PASSTHROUGH) {
    token = currentPrincipalToken() ?? "";
    if (!token) {
      if (allowBridgeIdentity) {
        token = AUTH_TOKEN;
      } else {
        const err = /** @type {HttpError} */ (
          new Error("no caller credential to forward upstream")
        );
        err.status = 401;
        throw err;
      }
    }
  }
  if (token) md.set("authorization", `Bearer ${token}`);
  // Propagate the inbound request id (or a generated one) so the server's
  // RPC span carries the same correlation id as the gateway access log.
  if (requestId) md.set("x-request-id", requestId);
  // Branch context (Phase 1: Isolation) travels as gRPC metadata, never as
  // a request-message field -- see service.proto's "Branch context" doc
  // comment. Plain ASCII string key, not `-bin`.
  if (branch) md.set("x-trogon-atlas-branch", branch);
  return md;
}

// AsyncLocalStorage carrying the current HTTP request's id and active
// branch (if any) through any downstream async hops (gRPC calls,
// paginateList loops, knowledge-graph fan-outs). Avoids threading
// `req.requestId` / `req.branch` through every helper.
/** @typedef {{ requestId?: string, branch?: string, principalToken?: string }} RequestContext */
const requestContext = new AsyncLocalStorage();

/** @returns {RequestContext} */
function currentContext() {
  return /** @type {RequestContext | undefined} */ (requestContext.getStore()) ?? {};
}

/**
 * Continue in the current context with some fields replaced. Rebuilding the
 * store from scratch instead would silently drop whatever the caller was not
 * thinking about, and one of those fields is the credential.
 *
 * @template T
 * @param {RequestContext} patch
 * @param {() => T} fn
 * @returns {T}
 */
function withContext(patch, fn) {
  return requestContext.run({ ...currentContext(), ...patch }, fn);
}

function currentRequestId() {
  return currentContext().requestId;
}

function currentBranch() {
  return currentContext().branch;
}

function currentPrincipalToken() {
  return currentContext().principalToken;
}

// Studio is an observation surface: agents make every change through CLI/MCP.
// An allowlist rather than a denylist, so an RPC added to the proto later is
// refused here until someone decides it is a read.
const READ_RPCS = new Set([
  "compileTypeLibrary",
  "diffBranch",
  "getEntity",
  "getServerInfo",
  "getTypeLibraryDescriptorSet",
  "listBranches",
  "listChanges",
  "listEntities",
  "listEntitiesByDomain",
  "listEntityKinds",
  "listNamespaces",
  "listValidationRules",
  "searchEntities",
  "validateEventModel",
  "validateProject",
]);

/** @param {string} method @param {unknown} request @param {{ timeoutMs?: number; requestId?: string; branch?: string; allowBridgeIdentity?: boolean }} [opts] */
function call(method, request, { timeoutMs = GRPC_DEADLINE_MS, requestId, branch, allowBridgeIdentity = false } = {}) {
  if (!READ_RPCS.has(method)) {
    const err = /** @type {HttpError} */ (new Error(`${method}: studio is read-only`));
    err.status = 403;
    return Promise.reject(err);
  }
  return new Promise((resolve, reject) => {
    const options = timeoutMs
      ? { deadline: new Date(Date.now() + timeoutMs) }
      : {};
    const effectiveId = requestId ?? currentRequestId();
    const effectiveBranch = branch ?? currentBranch();
    client[method](
      request,
      metadata(effectiveId, effectiveBranch, { allowBridgeIdentity }),
      options,
      (/** @type {unknown} */ err, /** @type {unknown} */ res) =>
        err ? reject(err) : resolve(res),
    );
  });
}

// Paginate a `listEntities`-shaped RPC up to MAX_PAGES. Throws a
// structured error if the upstream keeps returning a non-empty
// `nextPageToken` past the cap so the operator notices the runaway.
/** @param {(pageToken: string) => unknown} buildRequest */
async function paginateList(buildRequest) {
  /** @type {unknown[]} */
  const entities = [];
  let pageToken = "";
  let page = 0;
  do {
    if (page >= MAX_PAGES) {
      const err = /** @type {HttpError} */ (new Error(
        `gateway pagination cap reached: ${MAX_PAGES} pages of upstream data`,
      ));
      err.status = 502;
      throw err;
    }
    page += 1;
    const prevToken = pageToken;
    const resp = /** @type {any} */ (await call("listEntities", buildRequest(pageToken)));
    entities.push(...(resp.entities ?? []));
    pageToken = resp.nextPageToken ?? "";
    // A non-empty cursor that does not advance means the upstream is stuck;
    // continuing would burn MAX_PAGES identical RPCs for no progress.
    if (pageToken !== "" && pageToken === prevToken) {
      const err = /** @type {HttpError} */ (new Error(
        "gateway pagination stuck: upstream repeated the same nextPageToken",
      ));
      err.status = 502;
      throw err;
    }
  } while (pageToken !== "");
  return entities;
}

const BRANCH_QUERY_MAX_LENGTH = 402; // two 200-char segments + one '/'

/**
 * Parses and validates the optional branch context shared by every read
 * endpoint. Prefers `?branch=` (Studio URL idiom) and falls back to the
 * `x-trogon-atlas-branch` request header (same name the bridge forwards as
 * gRPC metadata). Returns `undefined` when absent (baseline,
 * byte-identical to pre-branching behavior). Throws a 400 HttpError on an
 * invalid name so a crafted branch name never reaches the gRPC metadata
 * layer.
 * @param {AppRequest} req
 */
function parseBranchQuery(req) {
  const headerRaw = req.headers?.["x-trogon-atlas-branch"];
  const headerValue = Array.isArray(headerRaw) ? headerRaw[0] : headerRaw;
  const raw = req.query?.branch ?? headerValue;
  if (raw == null || raw === "") return undefined;
  const value = String(raw);
  if (value.length > BRANCH_QUERY_MAX_LENGTH) {
    const err = /** @type {HttpError} */ (new Error(
      `branch query value too long (max ${BRANCH_QUERY_MAX_LENGTH} chars)`,
    ));
    err.status = 400;
    throw err;
  }
  if (!isSafeBranchName(value)) {
    const err = /** @type {HttpError} */ (new Error(`invalid branch name: ${JSON.stringify(value)}`));
    err.status = 400;
    throw err;
  }
  return value;
}

/**
 * Runs `fn` with the given branch (or none) attached to the async-local
 * request context, so every `call()` made during `fn` -- including
 * `paginateList` loops and multi-call fan-outs like /api/event-model --
 * forwards the same `x-trogon-atlas-branch` metadata without threading it
 * through each helper explicitly.
 * @template T
 * @param {AppRequest} req
 * @param {() => Promise<T>} fn
 */
function withBranch(req, fn) {
  const branch = parseBranchQuery(req);
  return withContext({ branch }, fn);
}

// The proto is compiled at runtime (protobufjs) so every Any embedded in
// entities (canonical annotations, scenario example payloads) decodes
// through real descriptors. google.protobuf well-known types (Struct, used
// as the pre-codegen payload convention) resolve from protobufjs' bundled
// definitions.
const protoBundleRoot = new protobuf.Root();
protoBundleRoot.resolvePath = (/** @type {string} */ _origin, /** @type {string} */ target_) => path.resolve(PROTO_INCLUDES[0], target_);
/** @type {protobuf.Root} */
let protoRoot;
try {
  protoRoot = await protoBundleRoot.load(PROTO_PATH);
} catch (err) {
  logEvent("error", "protobufjs_load_failed", {
    path: PROTO_PATH,
    error: maskError(err),
  });
  process.exit(1);
}

// Derive EntityKind lookup tables from the proto definitions at startup.
// Both the ENTITY_KIND_* -> number map (used for gRPC wire calls) and the
// snake_case shorthand -> number map (used in query-string parsing) are built
// from the single authoritative source so they cannot drift from the proto.
/** @type {Record<string, number>} */
const ENTITY_KIND_NUMBER = (() => {
  /** @type {protobuf.Enum | null} */
  let kindEnum = null;
  try {
    kindEnum = protoRoot.lookupEnum("trogonatlas.eventmodel.v1alpha1.EntityKind");
  } catch {
    // lookupEnum throws when not found.
  }
  if (!kindEnum || !kindEnum.values || Object.keys(kindEnum.values).length === 0) {
    logEvent("error", "proto_entity_kind_enum_missing", {
      msg: "trogonatlas.eventmodel.v1alpha1.EntityKind enum not found in proto -- cannot build kind tables",
      path: PROTO_PATH,
    });
    process.exit(1);
  }
  return /** @type {Record<string, number>} */ (kindEnum.values);
})();

// Snake_case shorthand map built from ENTITY_KIND_NUMBER:
// "ENTITY_KIND_EVENT" -> "event", etc. Strips the "ENTITY_KIND_" prefix and
// lowercases, so query-string values like "event_model" resolve to the number.
/** @type {Record<string, number>} */
const ENTITY_KIND_SNAKE = Object.fromEntries(
  Object.entries(ENTITY_KIND_NUMBER)
    .filter(([k]) => k.startsWith("ENTITY_KIND_"))
    .map(([k, v]) => [k.slice("ENTITY_KIND_".length).toLowerCase(), v]),
);

/**
 * Mirror `trogon_atlas_proto::canonical::parse_kind`: snake, dash, collapsed,
 * and ENTITY_KIND_ wire names; case-insensitive. Returns the numeric enum
 * value, or `null` when the token is unknown / UNSPECIFIED.
 *
 * @param {string} token
 * @returns {number | null}
 */
function parseKindToken(token) {
  let normalized = String(token).trim().toLowerCase().replaceAll("-", "_");
  if (normalized.startsWith("entity_kind_")) {
    normalized = normalized.slice("entity_kind_".length);
  }
  if (!normalized || normalized === "unspecified") return null;
  if (Object.hasOwn(ENTITY_KIND_SNAKE, normalized)) {
    const n = ENTITY_KIND_SNAKE[normalized];
    return n > 0 ? n : null;
  }
  const collapsed = normalized.replaceAll("_", "");
  for (const [key, n] of Object.entries(ENTITY_KIND_SNAKE)) {
    if (n > 0 && key.replaceAll("_", "") === collapsed) return n;
  }
  return null;
}

/** @param {any} value */
function structToJs(value) {
  // google.protobuf.Struct / Value / ListValue -> plain JSON
  if (value == null) return null;
  if (value.fields) {
    /** @type {Record<string, unknown>} */
    const out = {};
    for (const [k, v] of Object.entries(value.fields)) out[k] = valueToJs(v);
    return out;
  }
  return null;
}
/** @param {any} v */
function valueToJs(v) {
  if (!v || typeof v !== "object") return null;
  if ("nullValue" in v) return null;
  if ("numberValue" in v) return v.numberValue;
  if ("stringValue" in v) return v.stringValue;
  if ("boolValue" in v) return v.boolValue;
  if ("structValue" in v) return structToJs(v.structValue);
  if ("listValue" in v) return (v.listValue?.values ?? []).map(valueToJs);
  return null;
}

/** @param {string} typeUrl @param {unknown} value */
function decodeAnyValue(typeUrl, value) {
  try {
    const fqn = typeUrl.split("/").pop() ?? "";
    const type = protoRoot.lookupType(fqn);
    const buf = Buffer.isBuffer(value)
      ? value
      : Buffer.from(String(value ?? ""), "base64");
    const msg = type.decode(buf);
    if (fqn === "google.protobuf.Struct") {
      return { "@type": typeUrl, value: structToJs(type.toObject(msg)) };
    }
    const obj = type.toObject(msg, { longs: String, enums: String });
    return { "@type": typeUrl, ...obj };
  } catch (err) {
    logEvent("warn", "decode_any_value_failed", {
      typeUrl,
      error: String(/** @type {any} */ (err)?.message ?? err).slice(0, 200),
    });
    return null;
  }
}

const DECODE_ANNOTATIONS_MAX_DEPTH = 20;

/** @param {unknown} node @param {number} [depth] @returns {unknown} */
function decodeAnnotations(node, depth = 0) {
  if (depth >= DECODE_ANNOTATIONS_MAX_DEPTH) return node;
  if (Array.isArray(node)) {
    return node.map((item) => {
      const decoded = maybeDecodeAny(item);
      return decoded ?? decodeAnnotations(item, depth + 1);
    });
  }
  if (!node || typeof node !== "object" || Buffer.isBuffer(node)) return node;
  /** @type {Record<string, unknown>} */
  const out = {};
  for (const [k, v] of Object.entries(/** @type {Record<string, unknown>} */ (node))) {
    const decoded = maybeDecodeAny(v);
    out[k] = decoded ?? decodeAnnotations(v, depth + 1);
  }
  return out;
}

/** @param {unknown} item */
function maybeDecodeAny(item) {
  if (!item || typeof item !== "object" || Array.isArray(item)) return null;
  const url = /** @type {any} */ (item).typeUrl ?? /** @type {any} */ (item).type_url;
  if (typeof url !== "string" || !url.startsWith("type.googleapis.com/")) return null;
  return decodeAnyValue(url, /** @type {any} */ (item).value);
}

/** @type {Record<number, number>} */
const GRPC_TO_HTTP = {
  3: 400, // INVALID_ARGUMENT
  4: 504, // DEADLINE_EXCEEDED
  5: 404, // NOT_FOUND
  6: 409, // ALREADY_EXISTS
  7: 403, // PERMISSION_DENIED
  8: 429, // RESOURCE_EXHAUSTED
  9: 412, // FAILED_PRECONDITION
  10: 409, // ABORTED
  12: 501, // UNIMPLEMENTED
  14: 503, // UNAVAILABLE
  16: 401, // UNAUTHENTICATED
};

const app = express();
app.disable("x-powered-by");
app.set("trust proxy", false);

// Security headers: equivalent to the subset of `helmet` that matters
// for an API-only surface. Kept inline so the gateway has zero non-stdlib
// JS dependencies beyond what was already vendored.
app.use((/** @type {AppRequest} */ req, /** @type {Res} */ res, /** @type {Next} */ next) => {
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.setHeader("X-Frame-Options", "DENY");
  res.setHeader("Referrer-Policy", "no-referrer");
  // CORP defaults to same-origin; the CORS middleware below upgrades it to
  // cross-origin when an allowlisted Origin is actually granted, otherwise
  // browsers enforce CORP and defeat Access-Control-Allow-Origin.
  res.setHeader("Cross-Origin-Resource-Policy", "same-origin");
  res.setHeader("Cross-Origin-Opener-Policy", "same-origin");
  res.setHeader(
    "Permissions-Policy",
    "geolocation=(), microphone=(), camera=()",
  );
  // connect-src: always allow 'self' for the /api/* gateway. Add the NATS WS
  // origin only when TROGON_ATLAS_STUDIO_NATS_WS_URL is configured; bare `ws:`
  // and `wss:` scheme wildcards are intentionally absent (they allow any host).
  const connectSrc = NATS_WS_CSP_ORIGIN
    ? `'self' ${NATS_WS_CSP_ORIGIN}`
    : "'self'";
  res.setHeader(
    "Content-Security-Policy",
    `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ${connectSrc}; frame-ancestors 'none'`,
  );
  // HSTS is only meaningful over TLS; omit in non-production environments to
  // avoid locking out local HTTP dev flows.
  if (process.env.NODE_ENV === "production") {
    res.setHeader(
      "Strict-Transport-Security",
      "max-age=63072000; includeSubDomains",
    );
  }
  next();
});

// CORS: only the explicitly allowlisted origins are permitted. With an
// empty allowlist we emit no headers, so browsers enforce same-origin.
/** @param {AppRequest} req */
function corsOriginFor(req) {
  if (CORS_ORIGINS.length === 0) return undefined;
  if (CORS_ORIGINS.includes("*")) return "*";
  const requestOrigin = req.headers.origin;
  if (typeof requestOrigin !== "string") return undefined;
  return CORS_ORIGINS.includes(requestOrigin) ? requestOrigin : undefined;
}
app.use((/** @type {AppRequest} */ req, /** @type {Res} */ res, /** @type {Next} */ next) => {
  const allowed = corsOriginFor(req);
  if (allowed) {
    res.setHeader("Access-Control-Allow-Origin", allowed);
    res.setHeader("Vary", "Origin");
    res.setHeader("Access-Control-Allow-Methods", "GET, OPTIONS");
    res.setHeader(
      "Access-Control-Allow-Headers",
      "Content-Type, Authorization, X-Request-Id, x-trogon-atlas-branch",
    );
    res.setHeader("Access-Control-Max-Age", "600");
    // CORP same-origin + ACAO is a browser-level contradiction: relax CORP
    // only for the requests we are actually allowing cross-origin.
    res.setHeader("Cross-Origin-Resource-Policy", "cross-origin");
  }
  if (req.method === "OPTIONS") {
    return res.status(allowed ? 204 : 405).end();
  }
  next();
});

// Request id middleware: honor an inbound X-Request-Id (cap 128 chars) or
// generate one. Echoed back on the response so callers can correlate, and
// stored in AsyncLocalStorage so downstream gRPC calls propagate it
// automatically without threading the value through every helper.
app.use((/** @type {AppRequest} */ req, /** @type {Res} */ res, /** @type {Next} */ next) => {
  const inbound = req.headers["x-request-id"];
  const id =
    typeof inbound === "string" && inbound.length > 0 && inbound.length <= 128
      ? inbound
      : `gw-${Math.random().toString(36).slice(2, 10)}-${Date.now().toString(36)}`;
  /** @type {AppRequest} */ (req).requestId = id;
  res.setHeader("X-Request-Id", id);
  withContext({ requestId: id }, () => next());
});

// Sliding-window per-IP rate limiter (in-memory). Sufficient for a
// single-process gateway; multi-instance deployments should sit behind a
// shared limiter (e.g. NGINX) and can disable this by setting
// TROGON_ATLAS_STUDIO_RATE_MAX=0.
const RATE_BUCKETS = new Map();
const RATE_BUCKETS_MAX = 10_000;
/** @param {AppRequest} req @param {Res} res @param {Next} next */
const rateLimit = (req, res, next) => {
  if (RATE_LIMIT_MAX <= 0) return next();
  // `req.ip` falls back to the socket address when trust-proxy is off,
  // which is the right behavior for an internal gateway.
  const key = req.ip || req.socket?.remoteAddress || "unknown";
  const now = Date.now();
  const bucket = RATE_BUCKETS.get(key);
  if (!bucket || bucket.resetAt < now) {
    // Evict the least-recently-used entry (Map insertion order) when we hit
    // the cap so a unique-IP flood cannot grow this map without bound.
    if (!bucket && RATE_BUCKETS.size >= RATE_BUCKETS_MAX) {
      RATE_BUCKETS.delete(RATE_BUCKETS.keys().next().value);
    }
    RATE_BUCKETS.set(key, { count: 1, resetAt: now + RATE_LIMIT_WINDOW_MS });
    return next();
  }
  if (bucket.count >= RATE_LIMIT_MAX) {
    const retryAfter = Math.max(1, Math.ceil((bucket.resetAt - now) / 1000));
    res.setHeader("Retry-After", String(retryAfter));
    res.status(429).json({ error: "rate limit exceeded" });
    return;
  }
  // Refresh insertion order on hit so Map-order eviction behaves like LRU.
  RATE_BUCKETS.delete(key);
  bucket.count += 1;
  RATE_BUCKETS.set(key, bucket);
  next();
};

// Periodically drop expired buckets so memory does not grow unbounded
// under churn from short-lived client IPs. The try/catch keeps a mid-iter
// throw from killing the entire Node process.
setInterval(() => {
  try {
    const now = Date.now();
    for (const [key, bucket] of RATE_BUCKETS) {
      if (bucket.resetAt < now) RATE_BUCKETS.delete(key);
    }
  } catch (err) {
    logEvent("error", "rate_bucket_cleanup_failed", {
      error: maskError(err),
    });
  }
}, RATE_LIMIT_WINDOW_MS).unref?.();

// Structured access log: one JSON line per request.
app.use((/** @type {AppRequest} */ req, /** @type {Res} */ res, /** @type {Next} */ next) => {
  const appReq = /** @type {AppRequest} */ (req);
  const started = process.hrtime.bigint();
  res.on("finish", () => {
    const elapsedMs = Number(process.hrtime.bigint() - started) / 1e6;
    logEvent("info", "http_request", {
      method: req.method,
      path: req.path,
      status: res.statusCode,
      duration_ms: Math.round(elapsedMs),
      ip: req.ip,
      request_id: appReq.requestId,
    });
  });
  next();
});

// Digest a token to a fixed 32 bytes so the comparison below examines the
// same number of bytes every time. `timingSafeEqual` throws on a length
// mismatch, so comparing raw tokens needs a length guard, and that guard
// answers faster for a wrong-length guess -- which leaks how long the real
// token is. Hashing first removes the guard. This is a lookup key, not a
// password: the input is a high-entropy random token, so a plain digest is
// right and a slow KDF would only add latency. Mirrors `TokenDigest` in
// `rsworkspace/crates/trogon-atlas-server/src/auth.rs`.
/** @param {string} token */
function tokenDigest(token) {
  return createHash("sha256").update(token, "utf8").digest();
}

const AUTH_TOKEN_DIGEST = AUTH_TOKEN ? tokenDigest(AUTH_TOKEN) : null;

// Global bearer-token guard for all /api/* routes. /api/info is kept
// public (health/info endpoints are typically used by load balancers and
// monitoring probes that run before auth is available). When AUTH_TOKEN is
// not configured, all routes pass through.
/** @param {AppRequest} req @returns {string} */
function bearerToken(req) {
  const header = req.headers.authorization ?? "";
  return header.startsWith("Bearer ") ? header.slice(7).trim() : "";
}

/** @param {AppRequest} req @param {Res} res @param {Next} next */
function globalApiAuth(req, res, next) {
  // /api/info stays public regardless of token configuration.
  if (req.path === "/info") return next();
  const provided = bearerToken(req);
  if (AUTH_PASSTHROUGH) {
    // The bridge does not hold the token registry, so it cannot say whether
    // this credential is real -- only the server can, and it will. Checking
    // that one was presented keeps an anonymous request from silently
    // falling back to the bridge's own identity upstream.
    if (!provided) {
      res.status(401).json({ error: "unauthenticated" });
      return;
    }
    withContext({ principalToken: provided }, () => next());
    return;
  }
  if (!AUTH_TOKEN_DIGEST) return next();
  let authorized = false;
  try {
    authorized = timingSafeEqual(tokenDigest(provided), AUTH_TOKEN_DIGEST);
  } catch {
    authorized = false;
  }
  if (!authorized) {
    res.status(401).json({ error: "unauthenticated" });
    return;
  }
  next();
}

// Refuse every method that could carry a change, before any credential
// is checked. Studio observes; agents mutate through CLI/MCP, so no
// route here accepts a write and none that is added later can by accident.
// Compiling a type library is a dry run that writes nothing; it takes a
// POST only because the source text does not fit in a query string.
const OBSERVATION_METHODS = new Set(["GET", "HEAD", "OPTIONS"]);
const DRY_RUN_POSTS = new Set(["/type-libraries/compile"]);
app.use("/api", (/** @type {AppRequest} */ req, /** @type {Res} */ res, /** @type {Next} */ next) => {
  if (OBSERVATION_METHODS.has(req.method)) return next();
  if (req.method === "POST" && DRY_RUN_POSTS.has(req.path)) return next();
  res.status(403).json({ error: "studio is read-only; changes are made by agents through the CLI or MCP" });
});

app.use("/api", globalApiAuth);
app.use("/api", rateLimit);

/** @param {(req: AppRequest) => Promise<unknown> | unknown} fn */
function handler(fn) {
  return async (/** @type {AppRequest} */ req, /** @type {Res} */ res) => {
    const appReq = /** @type {AppRequest} */ (req);
    try {
      res.json(await fn(appReq));
    } catch (err) {
      const httpErr = /** @type {HttpError} */ (err);
      const status = httpErr.status ?? GRPC_TO_HTTP[/** @type {any} */ (err).code] ?? 500;
      logEvent("error", "handler_error", {
        path: req.path,
        status,
        request_id: appReq.requestId,
        error: maskError(err),
      });
      res.status(status).json({ error: maskError(err) });
    }
  };
}

// Like `handler`, but first parses+validates the shared `?branch=` query
// param and runs `fn` inside its async-local scope so every downstream
// `call()` (including paginateList loops and multi-call fan-outs)
// forwards it as `x-trogon-atlas-branch` metadata automatically. Absent
// `branch` = baseline, byte-identical to the pre-branching behavior.
/** @param {(req: AppRequest) => Promise<unknown> | unknown} fn */
function branchHandler(fn) {
  return async (/** @type {AppRequest} */ req, /** @type {Res} */ res) => {
    const appReq = /** @type {AppRequest} */ (req);
    try {
      res.json(await withBranch(appReq, () => Promise.resolve(fn(appReq))));
    } catch (err) {
      const httpErr = /** @type {HttpError} */ (err);
      const status = httpErr.status ?? GRPC_TO_HTTP[/** @type {any} */ (err).code] ?? 500;
      logEvent("error", "handler_error", {
        path: req.path,
        status,
        request_id: appReq.requestId,
        error: maskError(err),
      });
      res.status(status).json({ error: maskError(err) });
    }
  };
}

// Concurrency-limited map: runs `fn` over every item in `items` but keeps
// at most `concurrency` promises in-flight simultaneously. No new dependency.
/** @template T, U @param {T[]} items @param {number} concurrency @param {(item: T) => Promise<U>} fn @returns {Promise<U[]>} */
async function limitedMap(items, concurrency, fn) {
  /** @type {U[]} */
  const results = new Array(items.length);
  let index = 0;
  async function worker() {
    while (index < items.length) {
      const i = index++;
      results[i] = await fn(items[i]);
    }
  }
  const workers = Array.from({ length: Math.min(concurrency, items.length) }, worker);
  await Promise.all(workers);
  return results;
}

const GRPC_FANOUT_CONCURRENCY = 25;

// /api/nats-auth is covered by the global auth middleware when AUTH_TOKEN is
// set, so the per-route check here only adds an extra guard for the NATS
// token specifically (the global guard already rejected non-authed requests).
// If NATS_WS_AUTH_TOKEN is set but AUTH_TOKEN is not, the global auth guard
// is inactive and would expose the credential to any caller. Return 503 in
// that case to prevent a silent credential leak.
app.get("/api/nats-auth", async (/** @type {AppRequest} */ _req, /** @type {Res} */ res) => {
  // Credential responses must never be stored by shared caches or the
  // browser HTTP cache; even a null token would otherwise teach caches
  // to retain a later non-null body if the operator rotates config.
  res.setHeader("Cache-Control", "no-store");
  res.setHeader("Pragma", "no-cache");
  // In passthrough mode the route answers per caller, and the guard that let
  // the request in only established that the caller presented *a* token, not
  // a real one. Ask the server, which is the only party that knows.
  if (AUTH_PASSTHROUGH) {
    try {
      await call("getServerInfo", {});
    } catch (err) {
      const code = /** @type {any} */ (err)?.code;
      return res
        .status(GRPC_TO_HTTP[code] === 403 ? 403 : 401)
        .json({ error: "unauthenticated" });
    }
  }
  // Which realtime transport the browser is allowed to use.
  //
  // The NATS WebSocket listener grants one unscoped read of the entity
  // bucket to every client it admits: one shared token, one permission
  // set, no notion of who is connected. For a single-operator stack that
  // is a fine push channel. For a deployment that scopes namespaces per
  // principal it is a cross-tenant read, and it would hand every browser
  // the entities the gRPC lens just spent its effort hiding.
  //
  // So any bridge holding a credential says no, not just a pass-through
  // one. A fixed `AUTH_TOKEN` is a key that exists to scope something, and
  // the bucket read is not scoped to it: a bridge carrying one tenant's key
  // would still be sending browsers past it into every tenant's entities.
  // The bridge cannot ask how wide its own key is, so it assumes narrow.
  // Guessing wrong in this direction costs a poll instead of a push;
  // guessing wrong in the other costs every namespace on the deployment.
  //
  // `/api/changes/stream` carries the same feed through the server, which
  // means it is scoped to whoever is holding the key.
  //
  // A bridge with no credential of its own says no too, once the listener has
  // one: the browser cannot reach NATS without NATS_WS_AUTH_TOKEN, and this
  // endpoint is open in that configuration, so publishing it here would hand
  // the listener to exactly the drive-by it was closed against. Routing the
  // feed through the bridge keeps it working without that trade.
  if (AUTH_PASSTHROUGH || AUTH_TOKEN || NATS_WS_AUTH_TOKEN) {
    return res.json({ token: null, transport: "sse" });
  }
  res.json({ token: null, transport: "nats" });
});

app.get(
  "/api/info",
  // The only route exempt from `globalApiAuth` (see the guard above), so in
  // passthrough mode there is never a caller token to forward here. That is
  // the one case the bridge's own identity is allowed to stand in for.
  handler(() => call("getServerInfo", {}, { allowBridgeIdentity: true })),
);

// Static catalog of every validation rule the validator can emit. The
// payload mirrors `ListValidationRulesResponse`: `{ rules: [{ code,
// defaultSeverity, title, doc, subjectKind, subjectField, category }] }`.
// Stable across runs; the only thing that changes is the codebase version.
app.get(
  "/api/validation-rules",
  handler(() => call("listValidationRules", {})),
);

// Static catalog of every EntityKind the system stores (numeric, proto
// name, JSON discriminator, title, plural, doc).
app.get(
  "/api/entity-kinds",
  handler(() => call("listEntityKinds", {})),
);

// Validate every EventModel in a project or domain in one call. Query:
//   ?projectNamespace=foo&projectSlug=foo  -- scope by Project
//   ?domainNamespace=foo&domainSlug=foo    -- scope by Domain
app.get(
  "/api/validate-project",
  branchHandler(async (req) => {
    const q = req.query ?? {};
    // Validate all string query params with isSafeNamespace before use.
    for (const key of [
      "projectNamespace",
      "projectSlug",
      "projectVersion",
      "domainNamespace",
      "domainSlug",
      "domainVersion",
    ]) {
      if (q[key] != null) {
        const val = String(q[key]);
        if (val.length > NAMESPACE_QUERY_MAX_LENGTH) {
          const err = /** @type {HttpError} */ (new Error(`${key} query value too long (max ${NAMESPACE_QUERY_MAX_LENGTH} chars)`));
          err.status = 400;
          throw err;
        }
        if (!isSafeNamespace(val)) {
          const err = /** @type {HttpError} */ (new Error(`invalid value for ${key}: ${JSON.stringify(val)}`));
          err.status = 400;
          throw err;
        }
      }
    }
    const projectId = q.projectNamespace && q.projectSlug
      ? { namespace: String(q.projectNamespace), slug: String(q.projectSlug), version: String(q.projectVersion ?? "1") }
      : null;
    const domainId = q.domainNamespace && q.domainSlug
      ? { namespace: String(q.domainNamespace), slug: String(q.domainSlug), version: String(q.domainVersion ?? "1") }
      : null;
    if (!projectId && !domainId) {
      const err = /** @type {HttpError} */ (new Error("set projectNamespace+projectSlug OR domainNamespace+domainSlug"));
      err.status = 400;
      throw err;
    }
    const args = projectId ? { projectId } : { domainId };
    return await call("validateProject", args);
  }),
);

const PATH_PARAM_MAX = 512;

// List every entity scoped to a Domain via the server-side
// EM->BC->Subdomain->Domain walk. Query:
//   /api/by-domain/<namespace>/<slug>?kinds=event_model,bounded_context
app.get(
  "/api/by-domain/:namespace/:slug",
  branchHandler(async (req) => {
    const nsParam = String(req.params.namespace);
    const slugParam = String(req.params.slug);
    if (nsParam.length > PATH_PARAM_MAX || slugParam.length > PATH_PARAM_MAX) {
      const err = /** @type {HttpError} */ (new Error("path param too long"));
      err.status = 400;
      throw err;
    }
    if (!isSafeNamespace(nsParam)) {
      const err = /** @type {HttpError} */ (new Error(`invalid namespace: ${JSON.stringify(nsParam)}`));
      err.status = 400;
      throw err;
    }
    if (!isSafeNamespace(slugParam)) {
      const err = /** @type {HttpError} */ (new Error(`invalid slug: ${JSON.stringify(slugParam)}`));
      err.status = 400;
      throw err;
    }
    const kinds = [];
    if (req.query.kinds) {
      for (const raw of String(req.query.kinds).split(",").filter(Boolean)) {
        const n = parseKindToken(raw);
        if (n == null) {
          const err = /** @type {HttpError} */ (
            new Error(`unknown kind ${JSON.stringify(raw)}`)
          );
          err.status = 400;
          throw err;
        }
        kinds.push(n);
      }
    }
    return await call("listEntitiesByDomain", {
      domainId: {
        namespace: nsParam,
        slug: slugParam,
        version: "1",
      },
      kinds,
      latestVersionsOnly: true,
    });
  }),
);

// The namespace list the UI offers while filtered. Comes from the server's
// own ListNamespaces rather than from a scan of every event model, which
// this used to do: that scan could only see a namespace that already held a
// model, so one a client had just claimed through RegisterNamespace was
// missing from the picker until something was written into it.
//
// `namespaces` stays a list of NamespaceIds because that is what
// ListEntities filters on and what `?ns=` carries. `labels` maps each id to
// its human name, which is the only part safe to show: an id is either the
// name (a namespace adopted before the registry) or an opaque `ns_...`.
// Two owners may both name a namespace `orders`, so a name that is not
// unique across the response is qualified by its owner rather than printed
// twice.
app.get(
  "/api/namespaces",
  branchHandler(async () => {
    const resp = /** @type {any} */ (await call("listNamespaces", {}));
    const rows = (resp.namespaces ?? []).map((/** @type {any} */ n) => ({
      id: n.id || n.name,
      name: n.name || n.id,
      parent: n.parent ?? "",
    }));
    const nameCounts = new Map();
    for (const row of rows) {
      nameCounts.set(row.name, (nameCounts.get(row.name) ?? 0) + 1);
    }
    /** @type {Record<string, string>} */
    const labels = {};
    for (const row of rows) {
      labels[row.id] =
        nameCounts.get(row.name) > 1 && row.parent
          ? `${row.name} (${row.parent})`
          : row.name;
    }
    const namespaces = [...new Set(rows.map((/** @type {any} */ r) => r.id))].sort();
    return { namespaces, labels };
  }),
);

// The registry itself, as opposed to the picker above. Same RPC, but nothing
// is collapsed: the picker maps `id || name` so every namespace has something
// to filter on, and that mapping hides the one row worth seeing here.
//
// A namespace with entities but no registry row reports an empty `id`. The
// directory fails closed on those, so they are visible to an unrestricted
// caller and to nobody else: whoever holds a scoped key sees the namespace
// simply not exist. `registered: false` is how that reaches a person, and
// an agent claiming the name is the fix.
app.get(
  "/api/namespaces/registry",
  handler(async () => {
    const resp = /** @type {any} */ (await call("listNamespaces", {}));
    const namespaces = (resp.namespaces ?? []).map((/** @type {any} */ n) => ({
      id: n.id ?? "",
      name: n.name ?? "",
      parent: n.parent ?? "",
      entityCount: n.entityCount ?? 0,
      registered: Boolean(n.id),
    }));
    namespaces.sort((/** @type {any} */ a, /** @type {any} */ b) =>
      a.name.localeCompare(b.name) || a.parent.localeCompare(b.parent),
    );
    return { namespaces };
  }),
);

// List every branch (CreateBranch/ListBranches/DeleteBranch are exempt from
// branch-context resolution per the proto's "Branch context" doc comment,
// so this endpoint never forwards `x-trogon-atlas-branch` -- it always lists
// against the baseline registry).
app.get(
  "/api/branches",
  handler(async () => {
    const resp = /** @type {any} */ (await call("listBranches", {}));
    const branches = (resp.branches ?? []).map((/** @type {any} */ b) => ({
      name: b.name ?? "",
      doc: b.doc ?? "",
      createdAt: b.createdAt ?? "",
      deltaCount: b.deltaCount ?? 0,
    }));
    return { branches };
  }),
);

// Diff a branch against baseline for the review surface (three-pane
// conflict view). `name` is validated with the same branch-name rule as
// `?branch=` on the read endpoints. Entities embedded in each diff entry
// (base/ours/theirs) are decoded through the same Any-unwrapping path as
// every other entity envelope returned by this bridge.
app.get(
  "/api/branch-diff",
  handler(async (req) => {
    const raw = req.query?.name;
    if (raw == null || raw === "") {
      const err = /** @type {HttpError} */ (new Error("name query param is required"));
      err.status = 400;
      throw err;
    }
    const name = String(raw);
    if (name.length > BRANCH_QUERY_MAX_LENGTH) {
      const err = /** @type {HttpError} */ (new Error(
        `name query value too long (max ${BRANCH_QUERY_MAX_LENGTH} chars)`,
      ));
      err.status = 400;
      throw err;
    }
    if (!isSafeBranchName(name)) {
      const err = /** @type {HttpError} */ (new Error(`invalid branch name: ${JSON.stringify(name)}`));
      err.status = 400;
      throw err;
    }
    const resp = /** @type {any} */ (await call("diffBranch", { name }));
    const entries = (resp.entries ?? []).map((/** @type {any} */ e) => ({
      ref: e.ref ?? null,
      status: String(e.status ?? "STATUS_UNSPECIFIED"),
      base: e.base ? decodeAnnotations(e.base) : null,
      baseEtag: e.baseEtag ?? "",
      ours: e.ours ? decodeAnnotations(e.ours) : null,
      theirs: e.theirs ? decodeAnnotations(e.theirs) : null,
      conflictFieldPaths: e.conflictFieldPaths ?? [],
    }));
    return { entries };
  }),
);

const TYPE_LIBRARY_MAX_FILES = 256;

/** @param {string} message @returns {HttpError} */
function badRequest(message) {
  const err = /** @type {HttpError} */ (new Error(message));
  err.status = 400;
  return err;
}

/** @param {unknown} raw @param {string} where */
function parseLibraryId(raw, where) {
  const id = /** @type {Record<string, unknown> | null} */ (raw && typeof raw === "object" ? raw : null);
  const namespace = id?.namespace;
  const slug = id?.slug;
  const version = String(id?.version ?? "0");
  if (typeof namespace !== "string" || !isSafeNamespace(namespace)) throw badRequest(`${where}.namespace is invalid`);
  if (typeof slug !== "string" || !isSafeNamespace(slug)) throw badRequest(`${where}.slug is invalid`);
  if (!/^\d{1,20}$/.test(version)) throw badRequest(`${where}.version is invalid`);
  return { namespace, slug, version };
}

// Only the parts the compiler reads are forwarded, so a body cannot smuggle
// fields the bridge never meant to pass upstream.
/** @param {unknown} body */
function parseCompileBody(body) {
  const library = /** @type {Record<string, unknown> | undefined} */ (
    body && typeof body === "object" ? /** @type {Record<string, unknown>} */ (body).library : undefined
  );
  if (!library || typeof library !== "object") throw badRequest("library is required");
  const files = library.files;
  if (!Array.isArray(files) || files.length > TYPE_LIBRARY_MAX_FILES) throw badRequest("library.files is invalid");
  const deps = library.dependencies ?? [];
  if (!Array.isArray(deps) || deps.length > TYPE_LIBRARY_MAX_FILES) throw badRequest("library.dependencies is invalid");
  return {
    id: parseLibraryId(library.id, "library.id"),
    files: files.map((f, i) => {
      const { path: filePath, content } = /** @type {Record<string, unknown>} */ (f ?? {});
      if (typeof filePath !== "string" || typeof content !== "string") throw badRequest(`library.files[${i}] is invalid`);
      return { path: filePath, content };
    }),
    dependencies: deps.map((d, i) => ({
      id: parseLibraryId(/** @type {Record<string, unknown>} */ (d ?? {}).id, `library.dependencies[${i}].id`),
    })),
  };
}

app.post(
  "/api/type-libraries/compile",
  express.json({ limit: "4mb" }),
  branchHandler(async (req) => {
    const res = /** @type {{ diagnostics?: unknown[]; compatibilityViolations?: unknown[] }} */ (
      await call("compileTypeLibrary", { library: parseCompileBody(req.body) })
    );
    return { diagnostics: res.diagnostics ?? [], compatibilityViolations: res.compatibilityViolations ?? [] };
  }),
);

app.get(
  "/api/type-libraries/messages",
  branchHandler(async (req) => {
    const namespaces = parseNamespaceQuery(req);
    if (namespaces.length !== 1) throw badRequest("exactly one namespace is required");
    const res = /** @type {{ messages?: unknown[] }} */ (
      await call("getTypeLibraryDescriptorSet", { namespace: namespaces[0] })
    );
    return { messages: res.messages ?? [] };
  }),
);

/** @param {AppRequest} req @param {string} paramName */
function parseNamespaceQuery(req, paramName = "namespace") {
  const raw = req.query[paramName];
  if (!raw) return [];
  const str = String(raw);
  if (str.length > NAMESPACE_QUERY_MAX_LENGTH) {
    const err = /** @type {HttpError} */ (new Error(`${paramName} query value too long (max ${NAMESPACE_QUERY_MAX_LENGTH} chars)`));
    err.status = 400;
    throw err;
  }
  const parts = str.split(",").filter(Boolean);
  for (const part of parts) {
    if (!isSafeNamespace(part)) {
      const err = /** @type {HttpError} */ (new Error(`invalid namespace value: ${JSON.stringify(part)}`));
      err.status = 400;
      throw err;
    }
  }
  return parts;
}

app.get(
  "/api/model",
  branchHandler(async (req) => {
    // namespace accepts a comma-separated list; empty = all.
    const namespaces = parseNamespaceQuery(req);
    const entities = await paginateList((pageToken) => ({
      namespaces,
      latestVersionsOnly: true,
      pageSize: 500,
      pageToken,
    }));
    return { entities: decodeAnnotations(entities) };
  }),
);

// EntityKind enum -> numeric id (proto-loader decoded enums:String so the
// member kind comes back as 'ENTITY_KIND_EVENT_MODEL'; gRPC requires the
// numeric form on the wire). ENTITY_KIND_NUMBER is derived from the proto
// at startup above; this lookup function is the public interface.
/** @param {string | number} k */
const kindEnumToNumber = (k) => (typeof k === "number" ? k : ENTITY_KIND_NUMBER[k]);

// Overview landing data: problem-space entities, EventModels, and read models
// carrying subscription source refs for the context map.
app.get(
  "/api/overview",
  branchHandler(async () => {
    // READ_MODEL, EVENT_MODEL, BOUNDED_CONTEXT, DOMAIN, SUBDOMAIN
    const pages = await Promise.all(
      [3, 12, 16, 17, 18].map((kind) =>
        paginateList((pageToken) => ({
          kinds: [kind],
          latestVersionsOnly: true,
          pageSize: 500,
          pageToken,
        })),
      ),
    );
    return { entities: decodeAnnotations(pages.flat()) };
  }),
);

// Overview index data: just the EventModel entities. Stays cheap because we
// only load EVENT_MODEL kinds (numeric id 12 in the EntityKind enum).
app.get(
  "/api/event-models",
  branchHandler(async () => {
    const entities = await paginateList((pageToken) => ({
      kinds: [12],
      latestVersionsOnly: true,
      pageSize: 200,
      pageToken,
    }));
    return { entities: decodeAnnotations(entities) };
  }),
);

// One EventModel scoped to its members. Walks members, fetches each, and
// adds a halo of foreign-namespace events referenced as sourceEvents so
// the board can render cross-context stubs at the boundary.
app.get(
  "/api/event-model/:namespace/:slug",
  branchHandler(async (req) => {
    const ns = String(req.params.namespace);
    const slug = String(req.params.slug);
    if (ns.length > PATH_PARAM_MAX || slug.length > PATH_PARAM_MAX) {
      const err = /** @type {HttpError} */ (new Error("path param too long"));
      err.status = 400;
      throw err;
    }
    if (!isSafeNamespace(ns)) {
      const err = /** @type {HttpError} */ (new Error(`invalid namespace: ${JSON.stringify(ns)}`));
      err.status = 400;
      throw err;
    }
    if (!isSafeNamespace(slug)) {
      const err = /** @type {HttpError} */ (new Error(`invalid slug: ${JSON.stringify(slug)}`));
      err.status = 400;
      throw err;
    }
    const emResp = /** @type {any} */ (await call("getEntity", {
      kind: 12,
      id: { namespace: ns, slug, version: "1" },
    }));
    const em = emResp.entity;
    if (!em || !em.eventModel) {
      const err = /** @type {HttpError} */ (new Error(`event model not found: ${ns}/${slug}`));
      err.status = 404;
      throw err;
    }
    const members = em.eventModel.members ?? [];
    const wanted = new Map();
    for (const m of members) {
      const id = m.id;
      if (!id || !m.kind) continue;
      const numeric = typeof m.kind === "number" ? m.kind : kindEnumToNumber(m.kind);
      if (numeric == null) continue;
      wanted.set(
        `${numeric}:${id.namespace}/${id.slug}@${id.version}`,
        { kind: numeric, id },
      );
    }
    const wantedValues = [...wanted.values()];
    const fetched = await limitedMap(
      wantedValues,
      GRPC_FANOUT_CONCURRENCY,
      async (/** @type {{ kind: number, id: unknown }} */ { kind, id }) => {
        try {
          const resp = /** @type {any} */ (await call("getEntity", { kind, id }));
          return resp.entity;
        } catch (err) {
          logEvent("warn", "fanout_member_fetch_failed", { kind, id: /** @type {any} */ (id), error: maskError(err) });
          return null;
        }
      },
    );
    const entities = [em, ...fetched.filter(Boolean)];
    /** @type {Map<string, unknown>} */
    const haloIds = new Map();
    for (const ent of entities) {
      const rm = /** @type {any} */ (ent)?.readModel;
      if (!rm) continue;
      for (const src of rm.sourceEvents ?? []) {
        const id = src.id;
        if (!id || id.namespace === ns) continue;
        haloIds.set(`${id.namespace}/${id.slug}@${id.version}`, id);
      }
    }
    const halo = await limitedMap(
      [...haloIds.values()],
      GRPC_FANOUT_CONCURRENCY,
      async (/** @type {unknown} */ id) => {
        try {
          const resp = /** @type {any} */ (await call("getEntity", { kind: 1, id }));
          return resp.entity;
        } catch (err) {
          logEvent("warn", "fanout_halo_event_fetch_failed", { kind: 1, id: /** @type {any} */ (id), error: maskError(err) });
          return null;
        }
      },
    );
    // Knowledge-graph slice: the BoundedContext this model lives in,
    // the Subdomains it realizes, the parent Domain, and any sibling
    // BoundedContexts realizing those same subdomains. Lets the Domain
    // tab inside the EventModel page render a scoped chart of THIS
    // context's neighborhood instead of falling through to the
    // "No domains or subdomains in the store yet" empty state.
    const knowledge = [];
    try {
      const bcResp = /** @type {any} */ (await call("getEntity", {
        kind: 16, // BOUNDED_CONTEXT
        id: { namespace: ns, slug: ns, version: "1" },
      }));
      const bc = bcResp.entity;
      if (bc?.boundedContext) {
        knowledge.push(bc);
        const subdomainRefs = (bc.boundedContext.realizes ?? [])
          .map((/** @type {any} */ r) => r.id)
          .filter(Boolean);
        const subdomains = await limitedMap(
          subdomainRefs,
          GRPC_FANOUT_CONCURRENCY,
          async (/** @type {unknown} */ id) => {
            try {
              const r = /** @type {any} */ (await call("getEntity", { kind: 18, id }));
              return r.entity;
            } catch (err) {
              logEvent("warn", "fanout_subdomain_fetch_failed", { kind: 18, id: /** @type {any} */ (id), error: maskError(err) });
              return null;
            }
          },
        );
        /** @type {Map<string, unknown>} */
        const domainRefs = new Map();
        const subdomainKeys = new Set();
        for (const sd of subdomains) {
          if (!/** @type {any} */ (sd)?.subdomain) continue;
          knowledge.push(sd);
          subdomainKeys.add(`${/** @type {any} */ (sd).subdomain.id.namespace}/${/** @type {any} */ (sd).subdomain.id.slug}`);
          const did = /** @type {any} */ (sd).subdomain.domain?.id;
          if (did) domainRefs.set(`${did.namespace}/${did.slug}`, did);
        }
        // Parent Domain(s).
        const domains = await limitedMap(
          [...domainRefs.values()],
          GRPC_FANOUT_CONCURRENCY,
          async (/** @type {unknown} */ id) => {
            try {
              const r = /** @type {any} */ (await call("getEntity", { kind: 17, id }));
              return r.entity;
            } catch (err) {
              logEvent("warn", "fanout_domain_fetch_failed", { kind: 17, id: /** @type {any} */ (id), error: maskError(err) });
              return null;
            }
          },
        );
        for (const d of domains) if (d) knowledge.push(d);
        // Sibling BoundedContexts realizing any of the same subdomains.
        if (subdomainKeys.size > 0) {
          const allBcs = await paginateList((pageToken) => ({
            kinds: [16],
            latestVersionsOnly: true,
            pageSize: 500,
            pageToken,
          }));
          for (const sibling of allBcs) {
            const ent = /** @type {any} */ (sibling)?.boundedContext;
            if (!ent) continue;
            if (ent.id?.namespace === ns) continue;
            const touchesSubdomain = (ent.realizes ?? []).some((/** @type {any} */ r) =>
              r.id ? subdomainKeys.has(`${r.id.namespace}/${r.id.slug}`) : false,
            );
            if (touchesSubdomain) knowledge.push(sibling);
          }
        }
      }
    } catch (err) {
      logEvent("warn", "knowledge_graph_fetch_failed", {
        error: maskError(err),
      });
    }
    return {
      entities: decodeAnnotations([
        ...entities,
        ...halo.filter(Boolean),
        ...knowledge,
      ]),
    };
  }),
);

const SEARCH_QUERY_MAX_LENGTH = 1024;

app.get(
  "/api/search",
  branchHandler((req) => {
    const q = String(req.query.q ?? "");
    if (q.length > SEARCH_QUERY_MAX_LENGTH) {
      const err = /** @type {HttpError} */ (new Error(`q query value too long (max ${SEARCH_QUERY_MAX_LENGTH} chars)`));
      err.status = 400;
      throw err;
    }
    const namespaces = parseNamespaceQuery(req);
    return call("searchEntities", {
      query: q,
      namespaces,
      limit: 25,
    });
  }),
);

app.get(
  "/api/changes",
  branchHandler((req) => {
    const rawPageSize = req.query.pageSize ?? req.query.page_size;
    const parsed = rawPageSize != null ? Number(rawPageSize) : 100;
    const pageSize = Number.isInteger(parsed) && parsed > 0 ? Math.min(parsed, 500) : 100;
    return call("listChanges", {
      sinceToken: String(req.query.after ?? ""),
      pageSize,
    });
  }),
);

// SSE concurrency cap: long-poll connections hold one slot per client
// indefinitely, so the burst rate-limiter alone isn't enough. Cap the
// total number of in-flight SSE streams per source IP independently.
const SSE_CONNECTIONS = new Map();
// Tracks every active SSE response object so shutdown can drain them cleanly.
/** @type {Set<Res>} */
const activeSSEResponses = new Set();
const SSE_MAX_PER_IP = Number(
  process.env.TROGON_ATLAS_STUDIO_SSE_MAX_PER_IP ?? 4,
);

// SSE stream of the same change feed. Kept as a fallback (the browser
// now subscribes directly to NATS for realtime, see src/lib/realtime.ts,
// but this endpoint still works if WebSocket access is blocked).
// Auth is handled by the global middleware above; the per-IP cap below is
// an additional guard against connection exhaustion.
app.get("/api/changes/stream", async (/** @type {AppRequest} */ req, /** @type {Res} */ res) => {
  // The poll loop below outlives this handler's async context, so the
  // request's identity is captured here and re-entered on every poll.
  const sseContext = currentContext();
  const ip = req.ip || req.socket?.remoteAddress || "unknown";
  const active = SSE_CONNECTIONS.get(ip) ?? 0;
  if (active >= SSE_MAX_PER_IP) {
    res.setHeader("Retry-After", "30");
    res.status(429).json({ error: "too many concurrent SSE streams from this client" });
    return;
  }
  // Parse+validate ?branch= before opening the SSE stream so an invalid
  // name returns a JSON 400 instead of a half-open event stream.
  let branch;
  try {
    branch = parseBranchQuery(req);
  } catch (err) {
    const httpErr = /** @type {HttpError} */ (err);
    res.status(httpErr.status ?? 400).json({ error: maskError(err) });
    return;
  }
  // In pass-through mode the guard above only established that the caller
  // presented *a* token. Headers cannot be unsent once the stream is open,
  // so an unusable credential would become a 200 that emits nothing forever
  // rather than a 401 the browser can act on. Ask the server first, while a
  // status code is still ours to choose.
  if (AUTH_PASSTHROUGH) {
    try {
      await call("getServerInfo", {});
    } catch (err) {
      const code = /** @type {any} */ (err)?.code;
      res
        .status(GRPC_TO_HTTP[code] === 403 ? 403 : 401)
        .json({ error: "unauthenticated" });
      return;
    }
  }
  SSE_CONNECTIONS.set(ip, active + 1);
  res.setHeader("Content-Type", "text/event-stream");
  res.setHeader("Cache-Control", "no-cache");
  res.setHeader("Connection", "keep-alive");
  res.flushHeaders?.();
  activeSSEResponses.add(res);
  let token = String(
    req.headers["last-event-id"] ?? req.query.after ?? "",
  );
  let closed = false;
  /** @param {string} chunk */
  const safeWrite = (chunk) => {
    if (closed) return false;
    try {
      res.write(chunk);
      return true;
    } catch {
      closed = true;
      clearInterval(heartbeat);
      try {
        res.end();
      } catch {
        // Socket already gone.
      }
      return false;
    }
  };
  req.on("close", () => {
    closed = true;
    activeSSEResponses.delete(res);
    const n = (SSE_CONNECTIONS.get(ip) ?? 1) - 1;
    if (n <= 0) SSE_CONNECTIONS.delete(ip);
    else SSE_CONNECTIONS.set(ip, n);
  });
  // 15-second heartbeat keeps proxies (and EventSource itself) from
  // dropping the connection during idle periods.
  const heartbeat = setInterval(() => {
    if (closed) return;
    safeWrite(": keep-alive\n\n");
  }, 15_000);
  // Backoff state for consecutive upstream failures.
  const SSE_BACKOFF_BASE_MS = 1_000;
  const SSE_BACKOFF_CAP_MS = 30_000;
  const SSE_MAX_CONSECUTIVE_FAILURES = 10;
  let consecutiveFailures = 0;
  let backoffMs = SSE_BACKOFF_BASE_MS;

  try {
    while (!closed) {
      let page;
      try {
        // Run each poll inside the request's branch context so
        // `x-trogon-atlas-branch` is forwarded like GET /api/changes.
        page = /** @type {any} */ (
          await requestContext.run(
            { ...sseContext, branch },
            () =>
              call("listChanges", {
                sinceToken: token,
                pageSize: 100,
              }),
          )
        );
      } catch (err) {
        if (closed) break;
        consecutiveFailures += 1;
        if (!safeWrite(`event: error\ndata: ${JSON.stringify({ message: maskError(err) })}\n\n`)) break;
        if (consecutiveFailures >= SSE_MAX_CONSECUTIVE_FAILURES) {
          // Signal the client to stop and let EventSource handle reconnect
          // on its own schedule after the failure budget is exhausted.
          safeWrite(`event: fatal\ndata: ${JSON.stringify({ message: "upstream unavailable, closing stream" })}\n\n`);
          break;
        }
        await new Promise((r) => setTimeout(r, backoffMs));
        backoffMs = Math.min(backoffMs * 2, SSE_BACKOFF_CAP_MS);
        continue;
      }
      // Reset backoff on success.
      consecutiveFailures = 0;
      backoffMs = SSE_BACKOFF_BASE_MS;
      const events = page.events ?? [];
      const nextToken = page.nextToken ?? token;
      if (events.length > 0) {
        // One SSE message per page; the studio refetches the active
        // model when it receives any events. id: is the change token so
        // Last-Event-ID on reconnect resumes cleanly.
        if (
          !safeWrite(
            `id: ${nextToken}\ndata: ${JSON.stringify({ events, nextToken })}\n\n`,
          )
        ) {
          break;
        }
      }
      token = nextToken;
      // Quiet wait between polls -- short enough to feel realtime,
      // long enough to be polite to the upstream server.
      await new Promise((r) => setTimeout(r, 750));
    }
  } finally {
    clearInterval(heartbeat);
    activeSSEResponses.delete(res);
    if (!closed) res.end();
  }
});

app.get(
  "/api/validate",
  branchHandler((req) => {
    // Require non-empty, safe namespace and slug before forwarding upstream.
    for (const key of ["namespace", "slug"]) {
      const raw = req.query[key];
      if (raw == null || String(raw) === "") {
        const err = /** @type {HttpError} */ (new Error(`${key} is required`));
        err.status = 400;
        throw err;
      }
      const val = String(raw);
      if (val.length > NAMESPACE_QUERY_MAX_LENGTH) {
        const err = /** @type {HttpError} */ (new Error(`${key} query value too long (max ${NAMESPACE_QUERY_MAX_LENGTH} chars)`));
        err.status = 400;
        throw err;
      }
      if (!isSafeNamespace(val)) {
        const err = /** @type {HttpError} */ (new Error(`invalid value for ${key}: ${JSON.stringify(val)}`));
        err.status = 400;
        throw err;
      }
    }
    // version is optional (defaults to "1") but must be a safe id component
    // when present (same rule as validate-project's *Version params).
    const versionRaw = req.query.version;
    if (versionRaw != null && String(versionRaw) !== "") {
      const version = String(versionRaw);
      if (version.length > NAMESPACE_QUERY_MAX_LENGTH) {
        const err = /** @type {HttpError} */ (new Error(`version query value too long (max ${NAMESPACE_QUERY_MAX_LENGTH} chars)`));
        err.status = 400;
        throw err;
      }
      if (!isSafeNamespace(version)) {
        const err = /** @type {HttpError} */ (new Error(`invalid value for version: ${JSON.stringify(version)}`));
        err.status = 400;
        throw err;
      }
    }
    return call("validateEventModel", {
      eventModelId: {
        namespace: String(req.query.namespace),
        slug: String(req.query.slug),
        version: String(req.query.version ?? "1"),
      },
    });
  }),
);

// JSON 404 for anything under /api that no route handled. Must run before
// the SPA static fallback so `/api` and `/api/unknown` never return HTML.
app.use("/api", (/** @type {AppRequest} */ _req, /** @type {Res} */ res) => {
  res.status(404).json({ error: "not found" });
});

// Serve the built SPA when a dist/ exists (container mode); the Vite dev
// server takes this role in development.
const distDir = path.resolve(__dirname, "../dist");
if (fs.existsSync(distDir)) {
  app.use(express.static(distDir));
  // SPA shell only for document navigations. Script/module fetches send
  // Accept without text/html (often bare */*); returning index.html for
  // those yields MIME/"unexpected token '<'" failures on missing chunks.
  // Also exclude `/api` and `/api/*` (defense in depth with the JSON 404
  // above). The previous `^(?!\/api\/)` regex let bare `/api` through.
  app.get(/^(?!\/api(?:\/|$)).*/, (/** @type {AppRequest} */ req, /** @type {Res} */ res) => {
    const accept = req.headers.accept;
    const wantsHtml =
      typeof accept !== "string" ||
      accept === "" ||
      accept.split(",").some((part) => {
        const type = part.split(";")[0].trim().toLowerCase();
        return type === "text/html" || type === "application/xhtml+xml";
      });
    if (!wantsHtml) {
      res.status(404).end();
      return;
    }
    res.sendFile(path.join(distDir, "index.html"));
  });
}

app.use((/** @type {AppRequest} */ _req, /** @type {Res} */ res) => {
  res.status(404).end();
});

export { app, call };

if (process.env.NODE_ENV !== "test") {
  // Crash loudly rather than silently: an unhandled rejection that reaches
  // the process level means a code path escaped all try/catch boundaries.
  // Log it via the structured log channel so it lands in the same JSON stream
  // as all other events, then exit nonzero so the supervisor (Docker, systemd,
  // Kubernetes) knows to restart.
  process.on("unhandledRejection", (reason) => {
    logEvent("error", "unhandled_rejection", {
      error: maskError(reason),
    });
    process.exit(1);
  });

  process.on("uncaughtException", (err) => {
    logEvent("error", "uncaught_exception", {
      error: maskError(err),
    });
    process.exit(1);
  });

  const server = app.listen(PORT, HOST, () => {
    logEvent("info", "studio_bridge_listening", {
      host: HOST,
      port: PORT,
      upstream: maskEndpoint(GRPC_ENDPOINT),
    });
  });

  /** @param {string} signal */
  function shutdown(signal) {
    logEvent("info", "studio_bridge_shutdown", { signal, activeSSECount: activeSSEResponses.size });
    // Drain active SSE streams with a fatal event before closing the HTTP
    // server. Without this, server.close() hard-drops them mid-stream and
    // the browser EventSource cannot distinguish a clean close from a crash.
    for (const sseRes of activeSSEResponses) {
      try {
        sseRes.write(`event: fatal\ndata: ${JSON.stringify({ message: "server shutting down" })}\n\n`);
        sseRes.end();
      } catch {
        // Socket may already be gone; ignore.
      }
    }
    activeSSEResponses.clear();
    server.close(() => process.exit(0));
    setTimeout(() => process.exit(0), 10_000).unref?.();
  }
  process.on("SIGTERM", () => shutdown("SIGTERM"));
  process.on("SIGINT", () => shutdown("SIGINT"));

  // Crash loudly rather than silently: an unhandled rejection that reaches
  // the process level means a code path escaped all try/catch boundaries.
  // Log it via the structured log channel so it lands in the same JSON stream
  // as all other events, then exit nonzero so the supervisor (Docker, systemd,
  // Kubernetes) knows to restart.
  process.on("unhandledRejection", (reason) => {
    logEvent("error", "unhandled_rejection", {
      error: maskError(reason),
    });
    process.exit(1);
  });

  process.on("uncaughtException", (err) => {
    logEvent("error", "uncaught_exception", {
      error: maskError(err),
    });
    process.exit(1);
  });
}
