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

    expect(prompt).toMatch(/^Fix this validation finding.*Eventmodel CLI or MCP/);
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
          identity: 'command:shop/place-order@1',
          raw: expect.objectContaining({ doc: 'Place the order exactly once.' }),
        }),
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
      loadedButOmitted: [],
    });
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

    expect(prompt).toContain('commandSlice:shop/unrelated-slice@1');
    expect(prompt).not.toContain('storyboard:shop/unrelated-story@1');
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
});
