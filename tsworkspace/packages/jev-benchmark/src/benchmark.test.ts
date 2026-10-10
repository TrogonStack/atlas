import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { buildRequest, evaluate, fixturesSchema, optionsSchema, parseEvaluation, runBenchmark } from './benchmark.ts';

const fixtures = fixturesSchema.parse(
  JSON.parse(await readFile(new URL('./jev-field-mapping.json', import.meta.url), 'utf8')),
);
const request = buildRequest(fixtures[0]);
function response(choice = 'candidate_0', probability = 0.98) {
  const probabilities = {
    candidate_0: (1 - probability) / 2,
    candidate_1: (1 - probability) / 2,
    unresolved: (1 - probability) / 2,
  };
  probabilities[choice as keyof typeof probabilities] = probability;
  return {
    model: 'typesafe-ai/jev',
    answers: { source: { type: 'choice', choice, probabilities } },
    usage: { inputTokens: 300, outputTokens: 10 },
    providerMetadata: { gateway: { cost: '0.00001', generationId: 'not-exported' } },
  };
}
const fetchResponse =
  (body: unknown, status = 200): typeof fetch =>
  async () =>
    new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

test('synthetic fixtures validate and exclude expected labels from submitted state', () => {
  assert.equal(fixtures.length, 6);
  for (const fixture of fixtures) {
    const built = buildRequest(fixture);
    assert.equal(built.providerOptions.gateway.zeroDataRetention, true);
    assert.equal('expected' in built.state, false);
    assert.equal('id' in built.state, false);
    assert.ok(built.questions.source.criteria.unresolved);
  }
  assert.equal(fixturesSchema.safeParse([{ ...fixtures[0], expected: 'invented' }]).success, false);
  assert.equal(fixturesSchema.safeParse([fixtures[0], fixtures[0]]).success, false);
  assert.equal(
    fixturesSchema.safeParse([
      {
        ...fixtures[0],
        candidates: [fixtures[0].candidates[0], fixtures[0].candidates[0]],
      },
    ]).success,
    false,
  );
});

test('options bound cost, concurrency, threshold and request duration', () => {
  for (const options of [
    { repeats: 101 },
    { concurrency: 9 },
    { concurrency: 0 },
    { threshold: 1.01 },
    { threshold: -0.1 },
    { timeoutMs: 8001 },
    { timeoutMs: 0 },
  ])
    assert.equal(optionsSchema.safeParse(options).success, false);
});

test('evaluation validates candidate membership and probability distribution', () => {
  assert.equal(parseEvaluation(response(), request).answers.source.choice, 'candidate_0');
  for (const invalid of [
    { ...response(), model: 'unexpected/model' },
    { ...response(), answers: { source: { ...response().answers.source, choice: 'invented' } } },
    {
      ...response(),
      answers: { source: { type: 'choice', choice: 'candidate_0', probabilities: { candidate_0: 1 } } },
    },
    {
      ...response(),
      answers: {
        source: {
          type: 'choice',
          choice: 'candidate_0',
          probabilities: { candidate_0: 0.4, candidate_1: 0.5, unresolved: 0.1 },
        },
      },
    },
    {
      ...response(),
      answers: {
        source: {
          type: 'choice',
          choice: 'candidate_0',
          probabilities: { candidate_0: 1.1, candidate_1: 0, unresolved: 0 },
        },
      },
    },
    {
      ...response(),
      answers: {
        source: {
          type: 'choice',
          choice: 'candidate_0',
          probabilities: { candidate_0: 0.9, candidate_1: 0.9, unresolved: 0 },
        },
      },
    },
  ])
    assert.throws(() => parseEvaluation(invalid, request), /invalid_response/);
});

test('evaluation rejects incomplete probability mass despite a high-confidence selected choice', () => {
  assert.throws(
    () =>
      parseEvaluation(
        {
          ...response(),
          answers: {
            source: {
              type: 'choice',
              choice: 'candidate_0',
              probabilities: { candidate_0: 0.98, candidate_1: 0.01, unresolved: 0.005 },
            },
          },
        },
        request,
      ),
    /invalid_response/,
  );
});

test('evaluation accepts additive gateway confidence metadata without exposing it', () => {
  const upstream = response();
  const result = parseEvaluation(
    {
      ...upstream,
      answers: { source: { ...upstream.answers.source, confidence: 0.98 } },
    },
    request,
  );
  assert.equal(result.answers.source.choice, 'candidate_0');
  assert.equal('confidence' in result.answers.source, false);
});

test('fetch uses the fixed gateway endpoint and does not follow credential redirects', async () => {
  const fetcher: typeof fetch = async (url, init) => {
    assert.equal(url, 'https://ai-gateway.vercel.sh/v1/evaluate');
    assert.equal(init?.method, 'POST');
    assert.equal(init?.redirect, 'error');
    assert.deepEqual(JSON.parse(init?.body as string), request);
    assert.equal(new Headers(init?.headers).get('Authorization'), 'Bearer test-key');
    return new Response(JSON.stringify(response()));
  };
  const result = await evaluate(request, 'test-key', 100, fetcher);
  assert.equal(result.providerMetadata?.gateway?.cost, '0.00001');
  assert.equal('generationId' in (result.providerMetadata?.gateway ?? {}), false);
});

test('service errors never propagate response bodies or raw transport errors', async () => {
  for (const [status, code] of [
    [401, 'authentication'],
    [403, 'authentication'],
    [429, 'rate_limit'],
    [500, 'http_error'],
  ] as const) {
    await assert.rejects(
      evaluate(request, 'private-test-key', 100, fetchResponse({ error: 'private-test-key request body' }, status)),
      (error: Error) => error.message === code,
    );
  }
  await assert.rejects(
    evaluate(request, 'private-test-key', 100, async () => {
      throw new Error('private-test-key echoed by transport');
    }),
    (error: Error) => error.message === 'network_error',
  );
  await assert.rejects(
    evaluate(request, 'private-test-key', 100, async () => new Response('private-test-key')),
    (error: Error) => error.message === 'invalid_response',
  );
});

test('deadline bounds stalled fetch and stalled response bodies', async () => {
  let signal: AbortSignal | undefined;
  const fetcher: typeof fetch = async (_, init) => {
    signal = init?.signal ?? undefined;
    return await new Promise<Response>(() => {});
  };
  await assert.rejects(evaluate(request, 'test-key', 10, fetcher), /timeout/);
  assert.equal(signal?.aborted, true);
  await assert.rejects(
    evaluate(request, 'test-key', 10, async () => new Response(new ReadableStream({ start() {} }))),
    /timeout/,
  );
});

test('benchmark reports actual correctness, repeat count, errors and review outcomes', async () => {
  let call = 0;
  const fetcher: typeof fetch = async () => {
    const bodies = [response(), response('candidate_1'), response('unresolved'), response('candidate_0', 0.7)];
    return new Response(JSON.stringify(bodies[call++]));
  };
  const report = await runBenchmark([fixtures[0]], { repeats: 4, concurrency: 1 }, 'test-key', fetcher);
  assert.equal(report.scope, 'provider_only');
  assert.equal(report.repeats, 4);
  assert.equal(report.requests, 4);
  assert.equal(report.correct, 2);
  assert.equal(report.accuracy, 0.5);
  assert.equal(report.accepted, 2);
  assert.equal(report.incorrectAccepted, 1);
  assert.equal(report.review, 2);
  assert.equal(report.passed, false);
  assert.equal(report.samples[0].gatewayReportedCost, '0.00001');
  assert.equal(report.samples[0].usage?.inputTokens, 300);
  assert.ok(report.latencyMs.p95 >= report.latencyMs.p50);
  const errors = await runBenchmark([fixtures[0]], {}, 'test-key', fetchResponse({}, 401));
  assert.equal(errors.errors, 1);
  assert.equal(errors.accuracy, null);
  assert.equal(errors.passed, false);
  assert.equal(errors.review, 1);
  assert.equal(errors.samples[0].error, 'authentication');
});

test('benchmark bounds parallel requests and passes correct accepted choices', async () => {
  let active = 0;
  let maximum = 0;
  const fetcher: typeof fetch = async () => {
    active++;
    maximum = Math.max(active, maximum);
    await new Promise((resolve) => setTimeout(resolve, 5));
    active--;
    return new Response(JSON.stringify(response()));
  };
  const report = await runBenchmark([fixtures[0]], { repeats: 5, concurrency: 2 }, 'test-key', fetcher);
  assert.equal(maximum, 2);
  assert.equal(report.passed, true);
  assert.equal(report.correct, 5);
  assert.equal(report.accepted, 5);
  assert.equal(report.review, 0);
});
