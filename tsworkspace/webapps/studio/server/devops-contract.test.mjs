/**
 * Contract tests for studio ↔ rust proto/devops wiring.
 * These assert production-facing configuration; failures mean the stack
 * cannot boot or authenticate as documented.
 */
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { beforeAll, describe, expect, it } from 'vitest';
import protoLoader from '@grpc/proto-loader';
import grpc from '@grpc/grpc-js';

const here = path.dirname(fileURLToPath(import.meta.url));
const studioRoot = path.resolve(here, '..');
const atlasRoot = path.resolve(studioRoot, '..', '..', '..');
const fixturesCompose = path.join(atlasRoot, 'devops', 'docker', 'compose', 'dev', 'compose.yaml');
const fixturesOverride = path.join(atlasRoot, 'devops', 'docker', 'compose', 'dev', 'compose.override.yaml');
const chartDir = path.join(atlasRoot, 'devops', 'helm', 'charts', 'trogon-atlas');
const studioDockerfile = path.join(atlasRoot, 'devops', 'docker', 'images', 'trogon-atlas-studio', 'Dockerfile');

const HELM_TIMEOUT_MS = 30_000;

beforeAll(() => {
  execFileSync('helm', ['dependency', 'update', chartDir], { encoding: 'utf8', stdio: 'pipe' });
}, HELM_TIMEOUT_MS);

function loadServiceCtor(protoPath, includeDir) {
  const definition = protoLoader.loadSync(protoPath, {
    keepCase: false,
    longs: String,
    enums: String,
    defaults: false,
    oneofs: false,
    includeDirs: [includeDir ?? path.dirname(protoPath)],
  });
  const pkg = /** @type {any} */ (grpc.loadPackageDefinition(definition)).trogonatlas?.api?.eventmodel?.v1alpha1;
  return pkg?.EventModelService;
}

/** @param {string} filePath @param {string} py */
function pyEval(filePath, py) {
  return execFileSync(
    'python3',
    ['-c', py, filePath],
    { encoding: 'utf8' },
  ).trim();
}

const SERVICE_PKG_DIR = path.join('trogonatlas', 'api', 'eventmodel', 'v1alpha1');
const MODEL_PKG_DIR = path.join('trogonatlas', 'eventmodel', 'v1alpha1');

describe('proto entrypoint contract', () => {
  const protoRoot = path.resolve(atlasRoot, 'proto');

  it('service.proto exposes EventModelService (production Dockerfile PROTO_PATH)', () => {
    const ctor = loadServiceCtor(path.join(protoRoot, SERVICE_PKG_DIR, 'service.proto'), protoRoot);
    expect(typeof ctor).toBe('function');
  });

  it('event_model.proto does not expose EventModelService (must not be PROTO_PATH)', () => {
    // service definitions live in service.proto; using event_model.proto as
    // PROTO_PATH leaves EventModelService undefined at studio boot.
    const ctor = loadServiceCtor(path.join(protoRoot, MODEL_PKG_DIR, 'event_model.proto'), protoRoot);
    expect(typeof ctor).not.toBe('function');
  });
});

describe('dev compose PROTO_PATH', () => {
  it('base compose (the file RUNBOOK invokes with -f alone) points PROTO_PATH at a file that exposes EventModelService', () => {
    const protoPath = pyEval(
      fixturesCompose,
      'import sys,yaml; d=yaml.safe_load(open(sys.argv[1])); print(d["services"]["trogon-atlas-studio"]["environment"]["PROTO_PATH"])',
    );
    expect(protoPath).toBeTruthy();

    const hostProto = path.join(atlasRoot, 'proto', path.relative('/app/proto', protoPath));
    expect(fs.existsSync(hostProto), `missing ${hostProto}`).toBe(true);

    const ctor = loadServiceCtor(hostProto, path.resolve(atlasRoot, 'proto'));
    expect(
      typeof ctor,
      `PROTO_PATH=${protoPath} (host ${hostProto}) must define EventModelService; override-only fixes do not help RUNBOOK's docker compose -f …/compose.yaml`,
    ).toBe('function');
  });

  it('base compose and override both point PROTO_PATH at service.proto', () => {
    const base = pyEval(
      fixturesCompose,
      'import sys,yaml; d=yaml.safe_load(open(sys.argv[1])); print(d["services"]["trogon-atlas-studio"]["environment"]["PROTO_PATH"])',
    );
    const ovr = pyEval(
      fixturesOverride,
      'import sys,yaml; d=yaml.safe_load(open(sys.argv[1])); print(d["services"]["trogon-atlas-studio"]["environment"]["PROTO_PATH"])',
    );
    expect(base).toBe('/app/proto/trogonatlas/api/eventmodel/v1alpha1/service.proto');
    expect(ovr).toBe('/app/proto/trogonatlas/api/eventmodel/v1alpha1/service.proto');
  });
});

describe('helm studio auth with tokensFile', { timeout: HELM_TIMEOUT_MS }, () => {
  it('renders TROGON_ATLAS_AUTH_TOKEN for studio when server uses tokensFile and studio.auth.token is set', () => {
    const tokensFile = path.join(os.tmpdir(), `trogon-atlas-tokens-${process.pid}.toml`);
    fs.writeFileSync(
      tokensFile,
      `[[tokens]]\ntoken = "srv-secret"\nprincipal = "studio"\nroles = ["admin"]\n`,
    );
    try {
      const rendered = execFileSync(
        'helm',
        [
          'template',
          'test',
          chartDir,
          '--set-file',
          `server.auth.tokensFile.content=${tokensFile}`,
          '--set',
          'studio.auth.token.value=srv-secret',
          '--set',
          'server.config.natsUrl=nats://svc:pw@nats:4222',
        ],
        { encoding: 'utf8' },
      );
      const out = execFileSync(
        'python3',
        [
          '-c',
          `import sys,yaml
docs=list(yaml.safe_load_all(sys.stdin))
found=False
for d in docs:
  if not d: continue
  name=d.get("metadata",{}).get("name","")
  if d.get("kind")=="Deployment" and name.endswith("-studio"):
    env=d["spec"]["template"]["spec"]["containers"][0].get("env") or []
    found=any(e.get("name")=="TROGON_ATLAS_AUTH_TOKEN" for e in env)
print("yes" if found else "no")
`,
        ],
        { encoding: 'utf8', input: rendered },
      ).trim();
      expect(
        out,
        'studio.enabled defaults true; tokensFile is the recommended server auth; studio.auth.token must supply TROGON_ATLAS_AUTH_TOKEN',
      ).toBe('yes');
    } finally {
      fs.unlinkSync(tokensFile);
    }
  });

  it('template-fails when studio.enabled and tokensFile without studio.auth.token', () => {
    const tokensFile = path.join(os.tmpdir(), `trogon-atlas-tokens-fail-${process.pid}.toml`);
    fs.writeFileSync(
      tokensFile,
      `[[tokens]]\ntoken = "srv-secret"\nprincipal = "studio"\nroles = ["admin"]\n`,
    );
    try {
      let err;
      try {
        execFileSync(
          'helm',
          [
            'template',
            'test',
            chartDir,
            '--set-file',
            `server.auth.tokensFile.content=${tokensFile}`,
            '--set',
            'server.config.natsUrl=nats://svc:pw@nats:4222',
          ],
          { encoding: 'utf8' },
        );
      } catch (e) {
        err = e;
      }
      expect(err, 'helm template must fail without studio.auth.token').toBeTruthy();
      const msg = String(err.stderr || err.message || err);
      expect(msg).toMatch(/studio\.auth\.token|TROGON_ATLAS_AUTH_TOKEN/);
    } finally {
      fs.unlinkSync(tokensFile);
    }
  });

  it('template-fails when the server authenticates callers but reaches NATS anonymously', () => {
    // Ownership lives in a KV bucket in that store, so an open NATS is a way
    // around the server rather than a way into it: the lens keeps enforcing
    // against a registry anyone may rewrite.
    const tokensFile = path.join(os.tmpdir(), `trogon-atlas-tokens-store-${process.pid}.toml`);
    fs.writeFileSync(
      tokensFile,
      `[[tokens]]\ntoken = "srv-secret"\nprincipal = "studio"\nroles = ["admin"]\n`,
    );
    const base = [
      'template',
      'test',
      chartDir,
      '--set-file',
      `server.auth.tokensFile.content=${tokensFile}`,
      '--set',
      'studio.auth.token.value=srv-secret',
    ];
    try {
      let err;
      try {
        execFileSync('helm', base, { encoding: 'utf8' });
      } catch (e) {
        err = e;
      }
      expect(err, 'helm template must fail on a credential-less natsUrl').toBeTruthy();
      expect(String(err.stderr || err.message || err)).toMatch(/natsUrl carries no credentials/);

      // The escape hatch, for a store closed some other way.
      expect(() =>
        execFileSync('helm', [...base, '--set', 'server.config.acknowledgeUnauthenticatedStore=true'], {
          encoding: 'utf8',
        }),
      ).not.toThrow();
    } finally {
      fs.unlinkSync(tokensFile);
    }
  });
});

describe('helm spicedb wiring', { timeout: HELM_TIMEOUT_MS }, () => {
  const base = ['template', 'test', chartDir, '--set', 'server.auth.insecureAllowAnonymous=true'];

  function render(extra) {
    return execFileSync('helm', [...base, ...extra], { encoding: 'utf8' });
  }

  // helm only renders NOTES.txt on install, and `install --dry-run=client`
  // still calls the cluster. A throwaway manifest that includes the same
  // template keeps the assertion on the real notes without one.
  function renderNotes(extra) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'trogon-atlas-notes-'));
    try {
      const chartCopy = path.join(dir, 'trogon-atlas');
      fs.cpSync(chartDir, chartCopy, { recursive: true });
      fs.writeFileSync(
        path.join(chartCopy, 'templates', 'zz-notes-probe.yaml'),
        'apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: notes-probe\ndata:\n  notes: |\n{{ include "trogon-atlas/templates/NOTES.txt" . | indent 4 }}\n',
      );
      return execFileSync(
        'helm',
        [
          'template',
          'test',
          chartCopy,
          '-s',
          'templates/zz-notes-probe.yaml',
          '--set',
          'server.auth.insecureAllowAnonymous=true',
          ...extra,
        ],
        { encoding: 'utf8' },
      );
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  }

  function refusal(extra) {
    try {
      execFileSync('helm', [...base, ...extra], { encoding: 'utf8' });
    } catch (e) {
      return String(e.stderr || e.message || e);
    }
    return null;
  }

  const on = [
    '--set',
    'server.spicedb.endpoint=http://spicedb:50051',
    '--set',
    'server.spicedb.presharedKey.value=sekret',
  ];

  it('leaves the server on the registry authorizer by default, and says so', () => {
    // The gap this closes: before, a chart install could not turn SpiceDB on
    // at all, so every cluster silently answered visibility from the registry.
    const rendered = render([]);
    expect(rendered).not.toMatch(/TROGON_ATLAS_SPICEDB_ENDPOINT/);

    expect(renderNotes([])).toMatch(/registry's own parent column/);
  });

  it('passes the endpoint, the freshness and a secret-backed key to the server', () => {
    const rendered = render([...on, '--set', 'server.spicedb.freshness=fully-consistent']);
    expect(rendered).toMatch(/TROGON_ATLAS_SPICEDB_ENDPOINT/);
    expect(rendered).toMatch(/value: "fully-consistent"/);
    // The key reaches the container through a Secret reference, never as a
    // literal in the pod spec.
    expect(rendered).toMatch(/name: TROGON_ATLAS_SPICEDB_PRESHARED_KEY\n\s+valueFrom:/);
  });

  it('refuses a key with no endpoint, the way the server itself does', () => {
    // The server bails on exactly this pair. Rendering it would trade a
    // helm error for a CrashLoopBackOff.
    const msg = refusal(['--set', 'server.spicedb.presharedKey.value=sekret']);
    expect(msg).toMatch(/presharedKey is set without server\.spicedb\.endpoint/);
  });

  it('refuses an endpoint with no key', () => {
    const msg = refusal(['--set', 'server.spicedb.endpoint=http://spicedb:50051']);
    expect(msg).toMatch(/endpoint is set without server\.spicedb\.presharedKey/);
  });

  it('refuses a freshness the server would reject', () => {
    const msg = refusal([...on, '--set', 'server.spicedb.freshness=whenever']);
    expect(msg).toMatch(/not one of minimize-latency, fully-consistent/);
  });

  it('refuses to skip the startup sync with nothing else publishing grants', () => {
    // Startup sync and the token-file reload are one code path. Skipping it
    // means SpiceDB never learns about any principal, so every check answers
    // no and every caller sees an empty model.
    const msg = refusal([...on, '--set', 'server.spicedb.skipStartupSync=true']);
    expect(msg).toMatch(/nothing would ever publish grants/);

    expect(() =>
      render([...on, '--set', 'server.spicedb.skipStartupSync=true', '--set', 'server.spicedb.sync.enabled=true']),
    ).not.toThrow();
  });

  it('gives the sync CronJob the token file, because membership comes from there', () => {
    const tokensFile = path.join(os.tmpdir(), `trogon-atlas-tokens-spicedb-${process.pid}.toml`);
    fs.writeFileSync(tokensFile, `[principals.ci]\nrole = "writer"\ntokens = ["a"]\nparent = "acme"\n`);
    try {
      const rendered = execFileSync(
        'helm',
        [
          'template',
          'test',
          chartDir,
          '--set-file',
          `server.auth.tokensFile.content=${tokensFile}`,
          '--set',
          'studio.auth.token.value=t',
          '--set',
          'server.config.natsUrl=nats://svc:pw@nats:4222',
          ...on,
          '--set',
          'server.spicedb.sync.enabled=true',
        ],
        { encoding: 'utf8' },
      );
      const out = execFileSync(
        'python3',
        [
          '-c',
          `import sys,yaml
docs=list(yaml.safe_load_all(sys.stdin))
for d in docs:
  if not d: continue
  if d.get("kind")=="CronJob" and d["metadata"]["name"].endswith("-spicedb-sync"):
    spec=d["spec"]["jobTemplate"]["spec"]["template"]["spec"]
    c=spec["containers"][0]
    env={e["name"] for e in c.get("env") or []}
    mounts={m["name"] for m in c.get("volumeMounts") or []}
    print(",".join(c["args"]), "TROGON_ATLAS_AUTH_TOKENS_FILE" in env, "auth-tokens" in mounts)
`,
        ],
        { encoding: 'utf8', input: rendered },
      ).trim();
      expect(out).toBe('sync-spicedb True True');
    } finally {
      fs.unlinkSync(tokensFile);
    }
  });

  it('refuses a sync CronJob with nothing to sync to', () => {
    const msg = refusal(['--set', 'server.spicedb.sync.enabled=true']);
    expect(msg).toMatch(/sync\.enabled=true without server\.spicedb\.endpoint/);
  });
});

describe('studio CI lockfile contract', () => {
  const tsRoot = path.resolve(studioRoot, '..', '..');

  it('declares the tsworkspace as its own pnpm workspace', () => {
    // pnpm resolves the workspace from the nearest pnpm-workspace.yaml, so
    // tsworkspace must declare its own for installs to consult
    // tsworkspace/pnpm-lock.yaml.
    const workspace = fs.readFileSync(path.join(tsRoot, 'pnpm-workspace.yaml'), 'utf8');
    expect(workspace).toMatch(/^\s*-\s*"?webapps\/\*"?\s*$/m);
  });

  it('pins the same pnpm version for CI installs and the studio Dockerfile', () => {
    const miseToml = fs.readFileSync(path.join(atlasRoot, 'mise.toml'), 'utf8');
    const miseVersion = miseToml.match(/^pnpm\s*=\s*"([\d.]+)"/m)?.[1];
    expect(miseVersion, 'mise.toml must pin [tools] pnpm').toBeTruthy();
    expect(miseVersion).toBe('10.3.0');

    const dockerfile = fs.readFileSync(studioDockerfile, 'utf8');
    const dockerfileVersion = dockerfile.match(/corepack prepare pnpm@([\d.]+)/)?.[1];
    expect(dockerfileVersion, 'studio Dockerfile must pin a corepack pnpm version').toBeTruthy();
    expect(
      dockerfileVersion,
      'CI pnpm version (mise.toml) must match the studio Dockerfile corepack pin',
    ).toBe(miseVersion);
  });
});

describe('studio production Dockerfile lockfile contract', () => {
  it('stages pnpm-lock.yaml from the bind mount and installs with --frozen-lockfile before copying sources', () => {
    const dockerfile = fs.readFileSync(studioDockerfile, 'utf8');
    expect(
      dockerfile,
      'production image must pin deps to the same lockfile CI freezes against',
    ).toMatch(/cp\s+\/mnt\/tsworkspace\/pnpm-workspace\.yaml\s+\/mnt\/tsworkspace\/package\.json\s+\/mnt\/tsworkspace\/pnpm-lock\.yaml\s+\./);
    expect(dockerfile).toMatch(/pnpm install --frozen-lockfile/);
    // Install must happen while the lockfile is present; copying sources first
    // then installing without the lockfile reintroduces floating resolves.
    const installIdx = dockerfile.indexOf('pnpm install --frozen-lockfile');
    const copyAllIdx = dockerfile.search(/cp -r \/mnt\/tsworkspace\/\. \./);
    expect(installIdx).toBeGreaterThan(-1);
    expect(copyAllIdx).toBeGreaterThan(-1);
    expect(installIdx).toBeLessThan(copyAllIdx);
  });

  it('sets PROTO_PATH to service.proto (EventModelService entrypoint)', () => {
    const dockerfile = fs.readFileSync(studioDockerfile, 'utf8');
    expect(dockerfile).toMatch(/PROTO_PATH=\/app\/proto\/trogonatlas\/api\/eventmodel\/v1alpha1\/service\.proto/);
    expect(dockerfile).not.toMatch(/PROTO_PATH=\/app\/proto\/trogonatlas\/eventmodel\/v1alpha1\/event_model\.proto/);
  });
});

describe('dev compose studio image lockfile contract', () => {
  it('inline Dockerfile copies pnpm-lock.yaml and installs with --frozen-lockfile', () => {
    const compose = fs.readFileSync(fixturesCompose, 'utf8');
    expect(
      compose,
      'compose dockerfile_inline must install from the committed studio lockfile',
    ).toMatch(/COPY pnpm-workspace\.yaml package\.json pnpm-lock\.yaml \.\//);
    expect(compose).toMatch(/pnpm install --frozen-lockfile/);
  });
});

// The browser reaches NATS directly over the WebSocket listener, so that
// listener's grant is a production security boundary rather than a dev
// convenience. Both controls below are load-bearing and neither is
// sufficient alone: allowed_origins stops a drive-by page (a browser cannot
// forge Origin), while the scoped permissions stop any client that simply
// omits the header. See docs/explanation/authorization.md.
describe('nats websocket listener security contract', () => {
  const natsConf = fs.readFileSync(
    path.join(atlasRoot, 'devops', 'docker', 'compose', 'dev', 'services', 'nats', 'nats-server.conf'),
    'utf8',
  );
  // Strip comments first. This file documents its own directives in prose,
  // so matching the raw text would let a commented-out example satisfy an
  // assertion that the live config no longer satisfies.
  const stripComments = (/** @type {string} */ text) =>
    text
      .split('\n')
      .filter((line) => !line.trimStart().startsWith('#'))
      .join('\n');
  const wsBlock = (() => {
    const start = natsConf.indexOf('websocket {');
    expect(start, 'nats-server.conf must define a websocket block').toBeGreaterThan(-1);
    let depth = 0;
    for (let i = natsConf.indexOf('{', start); i < natsConf.length; i += 1) {
      if (natsConf[i] === '{') depth += 1;
      else if (natsConf[i] === '}') {
        depth -= 1;
        if (depth === 0) return stripComments(natsConf.slice(start, i + 1));
      }
    }
    throw new Error('unterminated websocket block');
  })();

  it('restricts which origins may open a websocket', () => {
    expect(
      wsBlock,
      'without allowed_origins any page the developer visits can open ws://127.0.0.1:8080',
    ).toMatch(/allowed_origins\s*:/);
  });

  it('does not use same_origin, which would reject the studio too', () => {
    // same_origin compares Origin against the request Host including port,
    // and the studio is served from a different port than NATS.
    expect(wsBlock).not.toMatch(/same_origin\s*:\s*true/);
  });

  it('gives browsers a different identity than the server', () => {
    const wsUser = wsBlock.match(/no_auth_user\s*:\s*([\w-]+)/)?.[1];
    const bare = stripComments(natsConf);
    const accountUser = bare
      .slice(bare.indexOf('}', bare.indexOf('authorization {')))
      .match(/^no_auth_user\s*:\s*([\w-]+)/m)?.[1];
    expect(wsUser, 'websocket block must pin its own no_auth_user').toBeTruthy();
    expect(
      wsUser,
      'browsers must not inherit the account-wide identity used by trogon-atlas-server',
    ).not.toBe(accountUser);
  });

  // OrbStack answers <service>.<project>.orb.local from the host whether or
  // not compose publishes a port, so the tcp listener is reachable from the
  // machine and an account-wide no_auth_user would hand `publish: ">"` to
  // anything running on it. Anonymous access has to stay scoped to the
  // websocket block, which grants the read-only identity.
  it('leaves no account-wide anonymous identity, so tcp clients must authenticate', () => {
    const bare = stripComments(natsConf);
    const outsideBlocks = bare
      .slice(bare.indexOf('}', bare.indexOf('authorization {')))
      .match(/^no_auth_user\s*:/m);
    expect(
      outsideBlocks,
      'an account-wide no_auth_user makes the tcp listener anonymous too',
    ).toBeNull();
  });

  it('requires a credential from the identity that may write', () => {
    const bare = stripComments(natsConf);
    const serviceUser = bare.match(/\{\s*user\s*:\s*service\b[^\n]*/)?.[0];
    expect(serviceUser, 'nats-server.conf must define the service user').toBeTruthy();
    expect(
      serviceUser,
      'the write-capable identity must carry a password, nkey, or token',
    ).toMatch(/(password|nkey|token)\s*:/);
  });

  it('denies the browser identity every write, so it cannot forge a registry row', () => {
    const wsUser = wsBlock.match(/no_auth_user\s*:\s*([\w-]+)/)?.[1];
    const bare = stripComments(natsConf);
    const userStart = bare.indexOf(`{ user: ${wsUser}`);
    expect(userStart, `nats-server.conf must define the ${wsUser} user`).toBeGreaterThan(-1);
    const publishAllow = bare
      .slice(userStart)
      .match(/publish\s*:\s*\{\s*allow\s*:\s*\[([\s\S]*?)\]/)?.[1];
    expect(publishAllow, 'the browser identity must use an explicit publish allow-list').toBeTruthy();
    const subjects = publishAllow.match(/"([^"]+)"/g)?.map((s) => s.slice(1, -1)) ?? [];
    expect(subjects.length).toBeGreaterThan(0);
    for (const subject of subjects) {
      // A KV write is a publish to $KV.<bucket>.<key>; ">" is everything.
      expect(subject, `browser may not publish to ${subject}`).not.toBe('>');
      expect(subject, `browser may not publish to ${subject}`).not.toMatch(/^\$KV\./);
    }
    expect(
      subjects.some((s) => s.startsWith('$JS.API.')),
      'the browser still needs JetStream API calls to create its watch consumer',
    ).toBe(true);
  });
});
