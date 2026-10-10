// Tests for entityContext, entitiesContext, and edgeContext in context.ts.
// All functions are pure transformations on model data: no side effects.

import { describe, expect, it } from 'vitest';
import { edgeContext, entitiesContext, entityContext } from './context';
import { buildIssueIndex } from './issues';
import type { BoardEdgeData } from './layout';
import { buildModel } from './model';

function mustFind<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected value to be present');
  return value;
}

const id = (namespace: string, slug: string, version = '1') => ({ namespace, slug, version });

function makeEntity(kind: string, namespace: string, slug: string) {
  return buildModel([{ [kind]: { id: id(namespace, slug), title: `${slug} title` } }] as never).entities[0];
}

describe('entityContext: basic structure', () => {
  it('includes the entity title and kind in the output', () => {
    const model = buildModel([{ event: { id: id('orders', 'placed'), title: 'Order placed' } }] as never);
    const entity = model.entities[0];
    const ctx = entityContext(model, entity);
    expect(ctx).toContain('Order placed');
    expect(ctx).toContain('kind: event');
  });

  it('includes the entity id in namespace/slug@vVersion format', () => {
    const model = buildModel([{ event: { id: id('orders', 'placed', '3'), title: 'Order placed' } }] as never);
    const entity = model.entities[0];
    const ctx = entityContext(model, entity);
    expect(ctx).toContain('orders/placed@v3');
  });

  it('includes doc section when entity has a doc', () => {
    const model = buildModel([
      { event: { id: id('orders', 'placed'), title: 'Order placed', doc: 'This is the doc' } },
    ] as never);
    const entity = model.entities[0];
    const ctx = entityContext(model, entity);
    expect(ctx).toContain('This is the doc');
  });

  it('includes raw JSON definition at the end', () => {
    const model = buildModel([{ event: { id: id('orders', 'placed'), title: 'Order placed' } }] as never);
    const entity = model.entities[0];
    const ctx = entityContext(model, entity);
    expect(ctx).toContain('## Raw definition');
    expect(ctx).toContain('```json');
  });

  it('returns a string that starts with the expected header', () => {
    const model = buildModel([{ event: { id: id('orders', 'placed'), title: 'Order placed' } }] as never);
    const entity = model.entities[0];
    const ctx = entityContext(model, entity);
    expect(ctx.trimStart()).toMatch(/^Context from an Event Modeling board/);
  });
});

describe('entityContext: slices touching section', () => {
  it('includes the "Slices touching" section when entity participates in a slice', () => {
    const model = buildModel([
      { event: { id: id('shop', 'ev'), title: 'Ev' } },
      { command: { id: id('shop', 'cmd'), title: 'Cmd' } },
      {
        commandSlice: {
          id: id('shop', 's1'),
          title: 'buy slice',
          command: { command: { id: id('shop', 'cmd') } },
          emittedEvents: [{ event: { id: id('shop', 'ev') } }],
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    const ctx = entityContext(model, event);
    expect(ctx).toContain('Slices touching this entity');
    expect(ctx).toContain('buy slice');
  });

  it('omits the slices section when entity has no slice membership', () => {
    const model = buildModel([{ event: { id: id('shop', 'lonely'), title: 'Lonely' } }] as never);
    const entity = model.entities[0];
    const ctx = entityContext(model, entity);
    expect(ctx).not.toContain('Slices touching this entity');
  });
});

describe('entityContext: fields section', () => {
  it('includes the fields section when entity has fields', () => {
    const model = buildModel([
      {
        event: {
          id: id('shop', 'ev'),
          title: 'Ev',
          schema: {
            '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema',
            fields: [{ name: 'orderId', type: { uuid: { version: 4 } } }],
          },
        },
      },
    ] as never);
    const entity = model.entities[0];
    const ctx = entityContext(model, entity);
    expect(ctx).toContain('## Fields');
    expect(ctx).toContain('orderId');
  });
});

describe('entitiesContext: bundle header', () => {
  it('wraps multiple entities with a bundle header and separator', () => {
    const model = buildModel([
      { event: { id: id('a', 'x'), title: 'X' } },
      { command: { id: id('a', 'y'), title: 'Y' } },
    ] as never);
    const ctx = entitiesContext(model, model.entities);
    expect(ctx).toMatch(/Context bundle: 2 entities/);
    expect(ctx).toContain('---');
  });

  it('returns a single-entity bundle without a separator', () => {
    const model = buildModel([{ event: { id: id('a', 'x'), title: 'X' } }] as never);
    const ctx = entitiesContext(model, model.entities);
    expect(ctx).toMatch(/Context bundle: 1 entities/);
  });

  it('returns an empty-like bundle for an empty entity list', () => {
    const model = buildModel([]);
    const ctx = entitiesContext(model, []);
    expect(ctx).toMatch(/Context bundle: 0 entities/);
  });
});

describe('edgeContext', () => {
  function makeMinimalEdge(): BoardEdgeData {
    const src = makeEntity('event', 'shop', 'placed');
    const tgt = makeEntity('command', 'shop', 'refund');
    return {
      relation: 'then',
      doc: '',
      metadata: [],
      source: src,
      target: tgt,
      slice: undefined,
    };
  }

  it('contains source and target titles', () => {
    const edge = makeMinimalEdge();
    const ctx = edgeContext(edge);
    expect(ctx).toContain('placed title');
    expect(ctx).toContain('refund title');
  });

  it('includes the relation label', () => {
    const edge = makeMinimalEdge();
    const ctx = edgeContext(edge);
    expect(ctx).toContain('relation:');
    expect(ctx).toContain('then');
  });

  it('starts with the Event Modeling board header', () => {
    const ctx = edgeContext(makeMinimalEdge());
    expect(ctx.trimStart()).toMatch(/^Context from an Event Modeling board/);
  });

  it('includes connection doc when present', () => {
    const edge = makeMinimalEdge();
    edge.doc = 'An important connection note';
    const ctx = edgeContext(edge);
    expect(ctx).toContain('An important connection note');
  });

  it('includes metadata section when metadata is non-empty', () => {
    const edge = makeMinimalEdge();
    edge.metadata = [{ key: 'owner', value: 'team-a' }];
    const ctx = edgeContext(edge);
    expect(ctx).toContain('## Connection metadata');
  });

  it('omits metadata section when metadata is empty', () => {
    const ctx = edgeContext(makeMinimalEdge());
    expect(ctx).not.toContain('## Connection metadata');
  });
});

// An empty GIVEN is a design claim ("this moment starts from no prior
// state"), so the line is rendered rather than dropped; otherwise a slice
// that deliberately starts a stream is indistinguishable from one whose
// preconditions nobody wrote down.
describe('entityContext: GIVEN is rendered even when empty', () => {
  const sliceWithScenario = (kind: string, scenario: Record<string, unknown>) =>
    buildModel([
      {
        [kind]: {
          id: id('supply-account', 'restore-transferred-units-slice'),
          title: 'Restore transferred units',
          scenarios: [scenario],
        },
      },
    ] as never);

  it('renders "GIVEN nothing" for a command slice scenario with no given', () => {
    const model = sliceWithScenario('commandSlice', {
      id: 'restore',
      title: 'Transfer compensated',
      when: { command: { id: id('supply-account', 'restore-transferred-units') } },
      emit: { events: [{ event: { id: id('supply-account', 'transfer.compensated') } }] },
    });
    const ctx = entityContext(model, mustFind(model.slices[0]).entity);
    expect(ctx).toContain('- GIVEN nothing');
    expect(ctx).toContain('- WHEN restore-transferred-units');
    expect(ctx).toContain('- THEN transfer.compensated');
  });

  it('renders "GIVEN nothing" for an automation slice scenario with no given', () => {
    const model = sliceWithScenario('automationSlice', {
      id: 'exhausted',
      title: 'Retries exhausted',
      whenDoc: 'retry budget exhausted',
      // biome-ignore lint/suspicious/noThenProperty: `then` is the AutomationScenario proto field
      then: { command: { id: id('supply-account', 'fail-transfer') } },
    });
    const ctx = entityContext(model, mustFind(model.slices[0]).entity);
    expect(ctx).toContain('- GIVEN nothing');
  });

  it('lists the real preconditions instead of "nothing" when a given exists', () => {
    const model = sliceWithScenario('commandSlice', {
      id: 'transferred',
      title: 'Already transferred',
      given: [{ event: { id: id('supply-account', 'transfer.requested') } }],
      when: { command: { id: id('supply-account', 'restore-transferred-units') } },
    });
    const ctx = entityContext(model, mustFind(model.slices[0]).entity);
    expect(ctx).toContain('- GIVEN transfer.requested');
    expect(ctx).not.toContain('- GIVEN nothing');
  });

  it('omits GIVEN entirely for a read model slice, whose inputs are the WHEN', () => {
    const model = sliceWithScenario('readModelSlice', {
      id: 'projects',
      title: 'Feed projects',
      when: [{ event: { id: id('supply-account', 'transfer.compensated') } }],
      // biome-ignore lint/suspicious/noThenProperty: `then` is the ReadModelScenario proto field
      then: { readModel: { id: id('supply-account', 'supplier-issuance') } },
    });
    const ctx = entityContext(model, mustFind(model.slices[0]).entity);
    expect(ctx).not.toContain('GIVEN');
    expect(ctx).toContain('- WHEN transfer.compensated');
  });
});

// A pasted context that omits the validator's findings describes a broken
// model as if it were healthy. The reader cannot tell "clean" from
// "nobody asked", so the section is rendered whenever findings exist.
describe('entityContext: validation findings', () => {
  const rmModel = () =>
    buildModel([
      {
        readModel: {
          id: id('registry', 'changesets-awaiting-review'),
          title: 'Changesets awaiting review',
          sourceEvents: [
            { event: { id: id('registry', 'changeset.submitted') } },
            { event: { id: id('registry', 'changeset.rejected') } },
          ],
        },
      },
    ] as never);

  const finding = {
    severity: 'SEVERITY_WARNING',
    code: 'RM_EVENT_NOT_PROJECTED',
    message: 'declares event registry/changeset.rejected@1 as a source but no in-model read-model slice projects it',
    subject: {
      kind: 'ENTITY_KIND_READ_MODEL',
      id: { namespace: 'registry', slug: 'changesets-awaiting-review', version: '1' },
    },
    subjectField: 'source_events',
    ruleTitle: 'Read model declares an event nothing projects',
    ruleCategory: 'model',
    ruleDefaultSeverity: 'SEVERITY_WARNING',
  };

  it('renders the code, severity, and structural field of each finding', () => {
    const model = rmModel();
    const entity = model.entities[0];
    const ctx = entityContext(model, entity, undefined, buildIssueIndex([finding]).for(entity));
    expect(ctx).toContain('## Validation');
    expect(ctx).toContain('- WARNING RM_EVENT_NOT_PROJECTED (source_events):');
    expect(ctx).toContain('no in-model read-model slice projects it');
  });

  it('omits the section entirely when the entity has no findings', () => {
    const model = rmModel();
    const entity = model.entities[0];
    expect(entityContext(model, entity, undefined, [])).not.toContain('## Validation');
    expect(entityContext(model, entity)).not.toContain('## Validation');
  });

  it('names the canonical severity when a rule downgraded itself for this case', () => {
    const model = rmModel();
    const entity = model.entities[0];
    const idx = buildIssueIndex([{ ...finding, severity: 'SEVERITY_INFO', ruleDefaultSeverity: 'SEVERITY_ERROR' }]);
    const ctx = entityContext(model, entity, undefined, idx.for(entity));
    expect(ctx).toContain('[rule default: error]');
  });

  it('threads per-entity findings through a bundle', () => {
    const model = rmModel();
    const ctx = entitiesContext(model, model.entities, buildIssueIndex([finding]));
    expect(ctx).toContain('- WARNING RM_EVENT_NOT_PROJECTED (source_events):');
  });
});
