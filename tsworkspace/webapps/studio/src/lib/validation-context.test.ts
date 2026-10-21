import { describe, expect, it } from 'vitest';
import type { Issue } from '@/lib/issues';
import { buildModel } from '@/lib/model';
import { validationFixPrompt } from '@/lib/validation-context';

const id = (slug: string, version = '1', namespace = 'shop') => ({ namespace, slug, version });

function finding(overrides: Partial<Issue> = {}): Issue {
  return {
    code: 'PROJECTION_BEFORE_EMITTER',
    severity: 'warning',
    message: 'shop/project-orders@1 projects shop/ordered@1 before its emitting command in shop/checkout@1.',
    field: 'source_events',
    ruleTitle: 'Projection must follow its emitter',
    ruleCategory: 'temporal',
    defaultSeverity: 'error',
    subject: { kind: 'readModelSlice', id: id('project-orders') },
    ...overrides,
  };
}

function snapshot(prompt: string, heading: string): unknown {
  const section = prompt.split(`## ${heading}\n\n`)[1];
  const json = section?.match(/```json\n([\s\S]*?)\n```/)?.[1];
  if (!json) throw new Error(`Missing JSON context for ${heading}`);
  return JSON.parse(json);
}

function temporalModel() {
  return buildModel([
    {
      event: {
        id: id('ordered'),
        title: 'Order placed',
        schema: {
          '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema',
          fields: [{ name: 'order_id' }],
        },
      },
    },
    { command: { id: id('place-order'), title: 'Place order', doc: 'Place the order exactly once.' } },
    { readModel: { id: id('orders'), title: 'Orders', sourceEvents: [{ id: id('ordered') }], key: ['order_id'] } },
    {
      commandSlice: {
        id: id('place-order-slice'),
        command: { command: { id: id('place-order') } },
        emittedEvents: [{ event: { id: id('ordered') } }],
      },
    },
    {
      readModelSlice: {
        id: id('project-orders'),
        sourceEvents: [{ event: { id: id('ordered') } }, { event: { id: id('payment-confirmed', '3', 'payments') } }],
        readModel: { readModel: { id: id('orders') } },
        projectionRole: 'PROJECTION_ROLE_UPSERT',
      },
    },
    {
      storyboard: {
        id: id('checkout'),
        slices: [{ id: id('project-orders') }, { id: id('place-order-slice') }, { id: id('project-orders') }],
      },
    },
    { event: { id: id('unrelated-event'), title: 'Unrelated event' } },
    { commandSlice: { id: id('unrelated-slice'), emittedEvents: [{ event: { id: id('unrelated-event') } }] } },
    { storyboard: { id: id('unrelated-story'), slices: [{ id: id('unrelated-slice') }] } },
  ]);
}

describe('validationFixPrompt', () => {
  it('requests an actual repair with current-state checks, preserved behavior, and validation results', () => {
    const issue = finding();
    const prompt = validationFixPrompt(temporalModel(), issue, {
      branch: 'fix-ordering',
      scopePath: '/em/shop/checkout',
    });

    expect(prompt).toMatch(
      /^Fix this validation finding in the Atlas project using the `trogon-atlas` CLI or the `trogon-atlas-mcp` MCP tools\./,
    );
    expect(prompt).toContain('Re-read the current model and validation results');
    expect(prompt).toContain('targeted repair that preserves intended behavior');
    expect(prompt).toContain('rerun validation');
    expect(prompt).toContain('report the entities changed');
    expect(prompt).not.toContain('\u2014');
    expect(snapshot(prompt, 'Scope')).toEqual({
      branch: 'fix-ordering',
      scopePath: '/em/shop/checkout',
      momentKey: null,
      namespaces: ['shop'],
    });
    expect(snapshot(prompt, 'Finding')).toEqual({
      code: issue.code,
      severity: issue.severity,
      message: issue.message,
      field: issue.field,
      subject: { identity: 'readModelSlice:shop/project-orders@1', ...issue.subject },
      rule: { title: issue.ruleTitle, category: issue.ruleCategory, defaultSeverity: 'error' },
    });
  });

  it('preserves full ordered storyboards, repeats, and causal definitions without unrelated model data', () => {
    const prompt = validationFixPrompt(temporalModel(), finding());

    expect(snapshot(prompt, 'Relevant loaded storyboards')).toEqual([
      expect.objectContaining({
        identity: 'storyboard:shop/checkout@1',
        orderedSlices: ['slice:shop/project-orders@1', 'slice:shop/place-order-slice@1', 'slice:shop/project-orders@1'],
        raw: {
          id: id('checkout'),
          slices: [{ id: id('project-orders') }, { id: id('place-order-slice') }, { id: id('project-orders') }],
        },
      }),
    ]);
    expect(snapshot(prompt, 'Loaded subject and related definitions')).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          identity: 'readModelSlice:shop/project-orders@1',
          raw: expect.objectContaining({ projectionRole: 'PROJECTION_ROLE_UPSERT' }),
        }),
        expect.objectContaining({ identity: 'commandSlice:shop/place-order-slice@1' }),
        expect.objectContaining({
          identity: 'event:shop/ordered@1',
          raw: expect.objectContaining({
            schema: expect.objectContaining({ fields: [{ name: 'order_id' }] }),
          }),
        }),
        expect.objectContaining({
          identity: 'readModel:shop/orders@1',
          raw: expect.objectContaining({ key: ['order_id'] }),
        }),
      ]),
    );
    expect(snapshot(prompt, 'References to fetch')).toEqual({
      notLoaded: ['event:payments/payment-confirmed@3'],
      loadedButOmitted: ['command:shop/place-order@1'],
    });
    expect(prompt).not.toContain('Place the order exactly once.');
    expect(prompt).not.toContain('unrelated');
  });

  it('identifies the exact missing subject and its containing storyboard without substituting another version', () => {
    const model = buildModel([
      { readModelSlice: { id: id('project-orders', '2'), doc: 'New version must not replace the pinned reference.' } },
      { storyboard: { id: id('checkout'), slices: [{ id: id('project-orders') }] } },
    ]);
    const prompt = validationFixPrompt(model, finding());

    expect(prompt).toContain('The subject is not loaded. Fetch its exact identity');
    expect(snapshot(prompt, 'References to fetch')).toEqual({
      notLoaded: ['readModelSlice:shop/project-orders@1', 'slice:shop/project-orders@1'],
      loadedButOmitted: [],
    });
    expect(prompt).toContain('storyboard:shop/checkout@1');
    expect(prompt).not.toContain('New version must not replace');
  });

  it('does not expand into another storyboard merely because it shares a neighboring slice', () => {
    const model = temporalModel();
    const storyboard = model.byKey.get('storyboard:shop/checkout@1');
    if (!storyboard) throw new Error('Expected checkout storyboard');
    storyboard.raw.slices = [...(storyboard.raw.slices as unknown[]), { id: id('unrelated-slice') }];

    const prompt = validationFixPrompt(model, finding());

    expect(snapshot(prompt, 'Relevant loaded storyboards')).toEqual([
      expect.objectContaining({
        orderedSlices: expect.arrayContaining(['slice:shop/unrelated-slice@1']),
      }),
    ]);
    expect(prompt).not.toContain('storyboard:shop/unrelated-story@1');
  });

  it('lists distant storyboard slices as ids to fetch instead of embedding their definitions', () => {
    const distant = Array.from({ length: 12 }, (_, index) => id(`distant-${index}`));
    const model = buildModel([
      ...temporalModel().entities.map((entity) => ({ [entity.kind]: entity.raw })),
      ...distant.map((slice) => ({ commandSlice: { id: slice, doc: `Distant slice ${slice.slug}.` } })),
    ]);
    const storyboard = model.byKey.get('storyboard:shop/checkout@1');
    if (!storyboard) throw new Error('Expected checkout storyboard');
    storyboard.raw.slices = [...(storyboard.raw.slices as unknown[]), ...distant.map((slice) => ({ id: slice }))];

    const prompt = validationFixPrompt(model, finding());

    const definitions = snapshot(prompt, 'Loaded subject and related definitions') as { identity: string }[];
    expect(definitions.map((definition) => definition.identity)).not.toContain('commandSlice:shop/distant-0@1');
    expect(prompt).not.toContain('Distant slice');
    const [board] = snapshot(prompt, 'Relevant loaded storyboards') as { orderedSlices: string[] }[];
    expect(board?.orderedSlices).toHaveLength(15);
    const fetch = snapshot(prompt, 'References to fetch') as { loadedButOmitted: string[] };
    expect(fetch.loadedButOmitted).toEqual(
      expect.arrayContaining(distant.map((slice) => `slice:shop/${slice.slug}@1`)),
    );
  });

  it('keeps the subject and its direct references but not references of references', () => {
    const model = buildModel([
      { event: { id: id('ordered'), doc: 'Direct event.' } },
      {
        readModel: {
          id: id('orders'),
          doc: 'Direct read model.',
          sourceEvents: [{ id: id('ordered') }, { id: id('far') }],
        },
      },
      { persona: { id: id('buyer'), doc: 'Direct persona.' } },
      { event: { id: id('far'), doc: 'Two hops away.' } },
      {
        readModelSlice: {
          id: id('project-orders'),
          sourceEvents: [{ event: { id: id('ordered') } }],
          readModel: { readModel: { id: id('orders') } },
          persona: { id: id('buyer') },
        },
      },
    ]);
    const prompt = validationFixPrompt(model, finding());
    const definitions = snapshot(prompt, 'Loaded subject and related definitions') as { identity: string }[];

    expect(definitions.map((definition) => definition.identity).sort()).toEqual([
      'event:shop/ordered@1',
      'persona:shop/buyer@1',
      'readModel:shop/orders@1',
      'readModelSlice:shop/project-orders@1',
    ]);
  });

  it('handles a model-wide finding using the baseline and root membership without dumping every member', () => {
    const model = buildModel([
      {
        eventModel: {
          id: id('checkout'),
          title: 'Checkout model',
          members: [{ kind: 'ENTITY_KIND_EVENT', id: id('ordered') }],
        },
      },
      { event: { id: id('ordered'), doc: 'Full event definition is outside the initial model-wide snapshot.' } },
    ]);
    const prompt = validationFixPrompt(model, finding({ subject: undefined, field: '', defaultSeverity: undefined }));

    expect(prompt).toContain('model-wide finding with no attached subject');
    expect(snapshot(prompt, 'Scope')).toEqual({
      branch: 'baseline',
      scopePath: null,
      momentKey: null,
      namespaces: ['shop'],
    });
    expect(snapshot(prompt, 'Finding')).toMatchObject({ subject: null, field: null, rule: { defaultSeverity: null } });
    expect(snapshot(prompt, 'References to fetch')).toEqual({
      notLoaded: [],
      loadedButOmitted: ['event:shop/ordered@1'],
    });
    expect(prompt).toContain('Checkout model');
    expect(prompt).not.toContain('Full event definition is outside');
  });

  it('includes a selected moment and flags an unloaded moment for fetching', () => {
    const prompt = validationFixPrompt(temporalModel(), finding({ subject: undefined }), {
      momentKey: 'commandSlice:shop/place-order-slice@1',
    });

    expect(prompt).toContain('storyboard:shop/checkout@1');
    expect(snapshot(prompt, 'Scope')).toMatchObject({ momentKey: 'commandSlice:shop/place-order-slice@1' });
    expect(
      validationFixPrompt(buildModel([]), finding({ subject: undefined }), {
        momentKey: 'commandSlice:shop/missing@1',
      }),
    ).toContain('The selected moment is not loaded: commandSlice:shop/missing@1');
  });

  it('does not invent references from scenario payload data that resembles an entity pointer', () => {
    const model = buildModel([
      {
        commandSlice: {
          id: id('place-order-slice'),
          scenarios: [
            {
              id: 'example',
              when: { command: { id: id('place-order') }, data: { event: { id: id('payload-only') } } },
            },
          ],
        },
      },
      { command: { id: id('place-order') } },
      { event: { id: id('payload-only'), doc: 'Do not treat payload data as a structural reference.' } },
    ]);
    const prompt = validationFixPrompt(
      model,
      finding({ subject: { kind: 'commandSlice', id: id('place-order-slice') } }),
    );

    expect(snapshot(prompt, 'References to fetch')).toEqual({ notLoaded: [], loadedButOmitted: [] });
    expect(prompt).not.toContain('Do not treat payload data as a structural reference.');
  });

  it.each(['OPEN_QUESTION_PRESENT', 'ASSUMPTION_PRESENT'])(
    'asks the human to settle a %s annotation instead of repairing the model',
    (code) => {
      const prompt = validationFixPrompt(temporalModel(), finding({ code, severity: 'info', field: 'metadata' }));

      expect(prompt).toMatch(/^Resolve this validation finding/);
      expect(prompt).toContain('Nothing in the model is broken.');
      expect(prompt).toContain('Ask the human');
      expect(prompt).toContain('edit the annotation or remove it once resolved');
      expect(prompt).not.toContain('targeted repair');
      expect(prompt).not.toContain('\u2014');
    },
  );

  it('keeps the repair instructions for structural findings', () => {
    const prompt = validationFixPrompt(
      temporalModel(),
      finding({ code: 'COMMAND_MULTIPLE_ISSUERS', subject: { kind: 'command', id: id('place-order') } }),
    );

    expect(prompt).toMatch(/^Fix this validation finding/);
    expect(prompt).toContain('targeted repair that preserves intended behavior');
    expect(prompt).not.toContain('Nothing in the model is broken.');
  });
});

describe('Specification validation repair context', () => {
  it('keeps nested exact targets and pins while excluding annotation lookalikes and retained text', () => {
    const model = buildModel([
      {
        specification: {
          id: id('orders'),
          title: 'Orders',
          schemas: [{ definition: { schema: { id: id('shape', '2') } } }],
          messages: [
            {
              schemas: [
                { libraryType: { library: { id: id('types', '3') }, message: { value: 'orders.Receipt' } } },
                {
                  contractType: {
                    schema: {
                      specification: { id: id('shared', '4') },
                      ownerUid: 'exact-owner',
                      memberId: { value: 'selected-schema' },
                    },
                    expectedArtifactId: { value: 'selected-artifact' },
                    expectedDigest: { sha256: 'a'.repeat(64) },
                  },
                },
              ],
              mapping: { readModel: { id: id('receipt', '2') } },
            },
          ],
          operations: [{ binding: { channel: { channel: { id: id('transport', '5') } } } }],
          exposures: [{ provider: { component: { id: id('gateway', '2') } } }],
          usages: [{ caller: { externalSystem: { id: id('caller', '3') } } }],
          dependencies: [{ specification: { id: id('dependency', '2') } }],
          metadata: [{ '@type': 'annotation', specification: { id: id('fake') } }],
          artifacts: [{ content: '{"specification":{"id":{"namespace":"shop","slug":"fake-text","version":"1"}}}' }],
        },
      },
    ]);
    const prompt = validationFixPrompt(
      model,
      finding({ code: 'SPECIFICATION_REFERENCE_UNRESOLVED', subject: { kind: 'specification', id: id('orders') } }),
    );
    expect(snapshot(prompt, 'References to fetch')).toEqual({
      notLoaded: [
        'schema:shop/shape@2',
        'typeLibrary:shop/types@3',
        'specification:shop/shared@4',
        'readModel:shop/receipt@2',
        'channel:shop/transport@5',
        'component:shop/gateway@2',
        'externalSystem:shop/caller@3',
        'specification:shop/dependency@2',
      ],
      loadedButOmitted: [],
    });
    expect(snapshot(prompt, 'Loaded subject and related definitions')).toEqual([
      expect.objectContaining({
        identity: 'specification:shop/orders@1',
        raw: expect.objectContaining({ messages: expect.any(Array) }),
      }),
    ]);
    expect(prompt).toContain('exact-owner');
    expect(prompt).toContain('selected-artifact');
  });
});

it('collects exact DataProduct nested data/type/access owners without following opaque metadata', () => {
  const model = buildModel([
    {
      dataProduct: {
        id: id('offering'),
        title: 'Offering',
        inputs: [{ data: { event: { id: id('fact', '3') } } }],
        outputs: [
          {
            data: { readModel: { id: id('receipt', '2') } },
            contract: { schema: { id: id('shape', '4') } },
            access: [
              {
                store: {
                  store: { id: id('cache', '5') },
                  attachment: { owner: { kind: 'ENTITY_KIND_DATA_STORE', id: id('cache', '5') } },
                },
              },
              {
                channel: {
                  channel: { id: id('transport', '6') },
                  observation: { owner: { kind: 'ENTITY_KIND_AUTOMATION_SLICE', id: id('observer', '7') } },
                },
              },
              { operation: { operation: { specification: { id: id('api', '8') } } } },
            ],
          },
        ],
        metadata: [{ data: { readModel: { id: id('secret') } } }],
      },
    },
  ]);
  const prompt = validationFixPrompt(model, finding({ subject: { kind: 'dataProduct', id: id('offering') } }));
  const refs = snapshot(prompt, 'References to fetch') as { notLoaded: string[] };
  expect(refs.notLoaded).toEqual(
    expect.arrayContaining([
      'event:shop/fact@3',
      'readModel:shop/receipt@2',
      'schema:shop/shape@4',
      'dataStore:shop/cache@5',
      'channel:shop/transport@6',
      'automationSlice:shop/observer@7',
      'specification:shop/api@8',
    ]),
  );
  expect(refs.notLoaded.join()).not.toContain('secret');
});

it('copies Team/Person membership and all exact responsibility selectors without following profile metadata', () => {
  const selectors = [
    { entity: { kind: 'ENTITY_KIND_TEAM', id: id('another-team', '2') } },
    {
      connection: {
        owner: { kind: 'ENTITY_KIND_AUTOMATION_SLICE', id: id('observer', '3') },
        ownerUid: 'edge-owner',
        id: { value: 'edge-b' },
      },
    },
    {
      member: {
        specification: { id: id('api', '4') },
        ownerUid: 'member-owner',
        kind: 'CONTRACT_MEMBER_KIND_ARTIFACT',
        memberId: { value: 'artifact-b' },
        expectedDigest: { sha256: 'a'.repeat(64) },
      },
    },
    {
      port: {
        product: { id: id('offering', '5') },
        ownerUid: 'port-owner',
        direction: 'DATA_PORT_DIRECTION_OUTPUT',
        portId: { value: 'output-b' },
      },
    },
  ];
  const model = buildModel([
    {
      team: {
        id: id('team'),
        title: { value: 'Team' },
        members: [{ person: { id: id('person', '6') }, connectionId: { value: 'membership-a' } }],
        responsibilities: selectors.map((subject, index) => ({
          connectionId: { value: `assignment-${index}` },
          kind: 'RESPONSIBILITY_KIND_CONTACT',
          subject,
        })),
        metadata: [{ person: { id: id('opaque-person') } }],
      },
    },
  ]);
  const prompt = validationFixPrompt(model, finding({ subject: { kind: 'team', id: id('team') } }));
  const refs = snapshot(prompt, 'References to fetch') as { notLoaded: string[] };
  expect(refs.notLoaded).toEqual(
    expect.arrayContaining([
      'team:shop/another-team@2',
      'automationSlice:shop/observer@3',
      'specification:shop/api@4',
      'dataProduct:shop/offering@5',
      'person:shop/person@6',
    ]),
  );
  expect(refs.notLoaded.join()).not.toContain('opaque-person');
  const definitions = snapshot(prompt, 'Loaded subject and related definitions') as {
    raw: { responsibilities: { subject: unknown }[] };
  }[];
  expect(definitions[0].raw.responsibilities.map((item) => item.subject)).toEqual(selectors);
});

describe('validationFixPrompt for strategic findings', () => {
  const sid = (namespace: string, slug: string) => id(slug, '1', namespace);
  const strategicModel = () =>
    buildModel([
      { project: { id: sid('acme', 'acme'), title: 'Acme' } },
      { domain: { id: sid('acme', 'acme'), title: 'Acme', project: { id: sid('acme', 'acme') } } },
      { domain: { id: sid('accounts', 'accounts'), title: 'Stray', doc: 'Declared in a context namespace.' } },
      { subdomain: { id: sid('acme', 'identity'), domain: { id: sid('acme', 'acme') }, classification: 'CORE' } },
      { subdomain: { id: sid('acme', 'billing'), domain: { id: sid('acme', 'acme') }, classification: 'GENERIC' } },
      { boundedContext: { id: sid('accounts', 'accounts'), realizes: [{ id: sid('acme', 'identity') }] } },
      { boundedContext: { id: sid('payments', 'payments'), realizes: [{ id: sid('acme', 'identity') }] } },
      { boundedContext: { id: sid('unrelated', 'unrelated') } },
    ]);
  const strategicFinding = (subject: NonNullable<Issue['subject']>, code = 'DOMAIN_IN_CONTEXT_NAMESPACE') =>
    finding({ code, subject, ruleCategory: 'strategic', field: '' });
  const loaded = (prompt: string) =>
    (snapshot(prompt, 'Loaded subject and related definitions') as { identity: string }[]).map((d) => d.identity);

  it('swaps the temporal guidance for the strategic repair instructions', () => {
    const prompt = validationFixPrompt(
      strategicModel(),
      strategicFinding({ kind: 'domain', id: sid('accounts', 'accounts') }),
    );
    expect(prompt).toContain('whose slug equals the Domain');
    expect(prompt).toContain('never create a Domain per context namespace');
    expect(prompt).toContain('split the context along classification lines instead of reclassifying');
    expect(prompt).toContain('Studio is a review surface');
    expect(prompt).toContain('`validate_project`');
    expect(prompt).toContain('docs/how-to/repair-validation-findings.md');
    expect(prompt).toContain('One problem space, contexts realize across namespaces');
    expect(prompt).not.toContain('For temporal findings');
    expect(prompt).not.toContain('\u2014');
  });

  it('keeps the temporal guidance for non-strategic findings', () => {
    const prompt = validationFixPrompt(temporalModel(), finding());
    expect(prompt).toContain('For temporal findings');
    expect(prompt).not.toContain('whose slug equals the Domain');
  });

  it('keeps the general repair guidance for a non-strategic finding on a strategic entity', () => {
    const prompt = validationFixPrompt(
      strategicModel(),
      finding({
        code: 'DANGLING_REF',
        ruleCategory: 'reference',
        field: 'realizes',
        subject: { kind: 'boundedContext', id: sid('accounts', 'accounts') },
      }),
    );
    expect(prompt).toContain('Apply a targeted repair that preserves intended behavior');
    expect(prompt).not.toContain('whose slug equals the Domain');
  });

  it('includes the contexts sharing a Domain namespace and the subdomains pointing at the Domain', () => {
    const prompt = validationFixPrompt(
      strategicModel(),
      strategicFinding({ kind: 'domain', id: sid('accounts', 'accounts') }),
    );
    expect(prompt).not.toContain('The subject is not loaded');
    expect(loaded(prompt)).toEqual(
      expect.arrayContaining(['domain:accounts/accounts@1', 'boundedContext:accounts/accounts@1']),
    );
    expect(loaded(prompt)).not.toContain('boundedContext:unrelated/unrelated@1');

    const parent = validationFixPrompt(strategicModel(), strategicFinding({ kind: 'domain', id: sid('acme', 'acme') }));
    expect(loaded(parent)).toEqual(
      expect.arrayContaining([
        'subdomain:acme/identity@1',
        'subdomain:acme/billing@1',
        'boundedContext:accounts/accounts@1',
        'boundedContext:payments/payments@1',
        'project:acme/acme@1',
      ]),
    );
    expect(loaded(parent)).not.toContain('boundedContext:unrelated/unrelated@1');
  });

  it('includes the contexts realizing a subdomain', () => {
    const prompt = validationFixPrompt(
      strategicModel(),
      strategicFinding({ kind: 'subdomain', id: sid('acme', 'identity') }, 'SUBDOMAIN_UNREALIZED'),
    );
    expect(loaded(prompt)).toEqual(
      expect.arrayContaining([
        'subdomain:acme/identity@1',
        'boundedContext:accounts/accounts@1',
        'boundedContext:payments/payments@1',
        'domain:acme/acme@1',
      ]),
    );
    expect(loaded(prompt)).not.toContain('boundedContext:unrelated/unrelated@1');
  });

  it('includes the realized subdomains, their domains, the stray domain and sibling contexts of a context', () => {
    const prompt = validationFixPrompt(
      strategicModel(),
      strategicFinding({ kind: 'boundedContext', id: sid('accounts', 'accounts') }, 'REALIZES_MIXED_CLASSIFICATIONS'),
    );
    expect(loaded(prompt)).toEqual(
      expect.arrayContaining([
        'boundedContext:accounts/accounts@1',
        'subdomain:acme/identity@1',
        'domain:acme/acme@1',
        'domain:accounts/accounts@1',
        'boundedContext:payments/payments@1',
      ]),
    );
    expect(loaded(prompt)).not.toContain('boundedContext:unrelated/unrelated@1');
  });

  it('treats the project reference of a domain as a reference to fetch', () => {
    const model = buildModel([{ domain: { id: sid('acme', 'acme'), project: { id: sid('acme', 'acme') } } }]);
    const prompt = validationFixPrompt(model, strategicFinding({ kind: 'domain', id: sid('acme', 'acme') }));
    expect(snapshot(prompt, 'References to fetch')).toEqual({
      notLoaded: ['project:acme/acme@1'],
      loadedButOmitted: [],
    });
  });
});
