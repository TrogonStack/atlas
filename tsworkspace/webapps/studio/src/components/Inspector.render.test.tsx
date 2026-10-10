// Render coverage for Inspector's main body: Inspector.test.tsx already
// covers safeLinkHref in isolation; this file exercises the component's
// section-by-section rendering (Timeline, Doc, Fields, Annotations,
// Scenarios, Slices membership, Tech, Problem space, Tracking, etc.) using
// realistic wire fixtures built through buildModel, following the fixture
// idiom in src/lib/model.test.ts (commandSliceFixture-style wire arrays).
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));

import type { WireEntityEnvelope } from '@/lib/api';
import { buildIssueIndex } from '@/lib/issues';
import { buildModel, type Entity } from '@/lib/model';
import { Inspector } from './Inspector';

const mkId = (ns: string, slug: string) => ({ namespace: ns, slug, version: '1' });

function mustFind(e: Entity | undefined): Entity {
  if (!e) throw new Error('fixture entity missing');
  return e;
}

function commandSliceFixture(): WireEntityEnvelope[] {
  const ns = 'orders';
  return [
    { ui: { id: mkId(ns, 'cart'), title: 'Cart' } },
    { command: { id: mkId(ns, 'place'), title: 'Place order' } },
    { event: { id: mkId(ns, 'placed'), title: 'Order placed', doc: 'The order was placed.' } },
    { event: { id: mkId(ns, 'rejected'), title: 'Order rejected' } },
    {
      commandSlice: {
        id: mkId(ns, 's-place'),
        title: 'place order',
        ui: { ui: { id: mkId(ns, 'cart') } },
        command: { command: { id: mkId(ns, 'place') } },
        emittedEvents: [{ event: { id: mkId(ns, 'placed') } }, { event: { id: mkId(ns, 'rejected') } }],
        scenarios: [
          {
            id: 'happy-path',
            title: 'Order placed successfully',
            given: [{ id: mkId(ns, 'cart'), data: { items: 2 } }],
            when: [{ id: mkId(ns, 'place'), data: { total: 42 } }],
            thenEmits: [{ id: mkId(ns, 'placed'), data: { orderId: 'o-1' } }],
          },
        ],
      },
    },
  ] as never;
}

function baseModel() {
  return buildModel(commandSliceFixture());
}

describe('Inspector', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders the entity title and identity', () => {
    const model = baseModel();
    const event = mustFind(model.entities.find((e) => e.id.slug === 'placed'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Order placed')).toBeDefined();
    expect(screen.getByText('orders/placed')).toBeDefined();
  });

  it('renders the doc section when doc is present', () => {
    const model = baseModel();
    const event = mustFind(model.entities.find((e) => e.id.slug === 'placed'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('The order was placed.')).toBeDefined();
  });

  it('renders the timeline with forward and backward neighbors', () => {
    const model = baseModel();
    const cmd = mustFind(model.entities.find((e) => e.kind === 'command'));
    render(<Inspector model={model} entity={cmd} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Timeline')).toBeDefined();
    expect(screen.getByText('Backward')).toBeDefined();
    expect(screen.getByText('Forward')).toBeDefined();
  });

  it('renders the instance id row when instanceUid is provided', () => {
    const model = baseModel();
    const event = mustFind(model.entities.find((e) => e.id.slug === 'placed'));
    render(<Inspector model={model} entity={event} instanceUid="uid-123" onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('uid-123')).toBeDefined();
  });

  it('renders the scenarios section for a slice-scoped entity', () => {
    const model = baseModel();
    const slice = mustFind(model.slices[0]?.entity);
    render(<Inspector model={model} entity={slice} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Scenarios (1)')).toBeDefined();
    expect(screen.getByText('Order placed successfully')).toBeDefined();
  });

  it('shows "given nothing" when a command-slice scenario declares no preconditions', () => {
    const wire = commandSliceFixture();
    const slice = wire[wire.length - 1] as unknown as {
      commandSlice: { scenarios: Record<string, unknown>[] };
    };
    delete slice.commandSlice.scenarios[0].given;
    const model = buildModel(wire);
    const entity = mustFind(model.slices[0]?.entity);
    render(<Inspector model={model} entity={entity} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Given')).toBeDefined();
    expect(screen.getByText('nothing')).toBeDefined();
  });

  it('omits the Given section for a read model slice, whose inputs are the When', () => {
    const ns = 'orders';
    const model = buildModel([
      { event: { id: mkId(ns, 'placed'), title: 'Order placed' } },
      { readModel: { id: mkId(ns, 'orders-feed'), title: 'Orders feed' } },
      {
        readModelSlice: {
          id: mkId(ns, 's-feed'),
          title: 'feed projects',
          readModel: { readModel: { id: mkId(ns, 'orders-feed') } },
          sourceEvents: [{ event: { id: mkId(ns, 'placed') } }],
          scenarios: [{ id: 'projects', title: 'Feed projects', when: [{ id: mkId(ns, 'placed') }] }],
        },
      },
    ] as never);
    const entity = mustFind(model.slices[0]?.entity);
    render(<Inspector model={model} entity={entity} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Scenarios (1)')).toBeDefined();
    expect(screen.queryByText('Given')).toBeNull();
  });

  it('renders the slices membership section and navigates on click', () => {
    const model = baseModel();
    const onSelect = vi.fn();
    const cmd = mustFind(model.entities.find((e) => e.kind === 'command'));
    render(<Inspector model={model} entity={cmd} onClose={vi.fn()} onSelect={onSelect} />);
    expect(screen.getByText(/Slices \(\d+\)/)).toBeDefined();
    fireEvent.click(screen.getByText('command slice'));
    expect(onSelect).toHaveBeenCalled();
  });

  it('calls onClose when the close button is clicked', () => {
    const model = baseModel();
    const onClose = vi.fn();
    const event = mustFind(model.entities.find((e) => e.id.slug === 'placed'));
    render(<Inspector model={model} entity={event} onClose={onClose} onSelect={vi.fn()} />);
    const buttons = screen.getAllByRole('button');
    const closeButton = buttons.find((b) => !b.textContent?.includes('Copy') && b.querySelector('svg'));
    expect(closeButton).toBeDefined();
    fireEvent.click(closeButton!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('renders the role badge when role is set', () => {
    const model = buildModel([{ ui: { id: mkId('orders', 'cart'), title: 'Cart', role: 'primary' } }] as never);
    const ui = mustFind(model.entities.find((e) => e.kind === 'ui'));
    render(<Inspector model={model} entity={ui} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Role')).toBeDefined();
    expect(screen.getByText('primary')).toBeDefined();
  });

  it('renders the stream identity section when streamId is set', () => {
    const model = buildModel([
      { event: { id: mkId('orders', 'placed'), title: 'Order placed', streamId: 'order-{orderId}' } },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Stream identity')).toBeDefined();
    expect(screen.getByText('order-{orderId}')).toBeDefined();
  });

  it('renders the external-source section', () => {
    const model = buildModel([
      {
        readModel: {
          id: mkId('orders', 'summary'),
          title: 'Order summary',
          externalSource: { system: { id: { slug: 'billing' } }, descriptor: 'webhook' },
        },
      },
    ] as never);
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(<Inspector model={model} entity={rm} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('External source')).toBeDefined();
    expect(screen.getByText('billing')).toBeDefined();
  });

  it('renders the tech badges', () => {
    const model = buildModel([
      { readModel: { id: mkId('orders', 'summary'), title: 'Order summary', tech: ['postgres', 'redis'] } },
    ] as never);
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(<Inspector model={model} entity={rm} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Tech')).toBeDefined();
    expect(screen.getByText('postgres')).toBeDefined();
    expect(screen.getByText('redis')).toBeDefined();
  });

  it('renders the fields section', () => {
    const model = buildModel([
      {
        event: {
          id: mkId('orders', 'placed'),
          title: 'Order placed',
          schema: {
            '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema',
            fields: [{ name: 'orderId', type: { primitive: 'STRING' }, doc: 'the order id' }],
          },
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Fields (1)')).toBeDefined();
    expect(screen.getByText('orderId')).toBeDefined();
  });

  it('renders the orphan section with a filled-in doc', () => {
    const model = buildModel([
      {
        event: {
          id: mkId('orders', 'placed'),
          title: 'Order placed',
          metadata: [
            {
              '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.OrphanAnnotation',
              doc: 'intentionally unused',
            },
          ],
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Orphan')).toBeDefined();
    expect(screen.getByText('intentionally unused')).toBeDefined();
  });

  it('renders the annotations section for a note annotation', () => {
    const model = buildModel([
      {
        event: {
          id: mkId('orders', 'placed'),
          title: 'Order placed',
          metadata: [
            {
              '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.NoteAnnotation',
              text: 'remember to check this',
              author: 'ada',
            },
          ],
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Annotations')).toBeDefined();
    expect(screen.getByText('remember to check this')).toBeDefined();
    expect(screen.getByText('(ada)')).toBeDefined();
  });

  it('renders the annotations section for a failure mode annotation', () => {
    const model = buildModel([
      {
        event: {
          id: mkId('orders', 'placed'),
          title: 'Order placed',
          metadata: [
            {
              '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.FailureModeAnnotation',
              trigger: 'network disconnect',
              behavior: 'shows a banner and freezes the last good render',
              recovery: 'auto-reconnect with backoff',
            },
          ],
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Annotations')).toBeDefined();
    expect(screen.getByText('network disconnect')).toBeDefined();
    expect(screen.getByText('shows a banner and freezes the last good render')).toBeDefined();
    expect(screen.getByText('auto-reconnect with backoff')).toBeDefined();
  });

  it('renders a code ref annotation as repo, path and symbol rather than raw json', () => {
    const model = buildModel([
      {
        event: {
          id: mkId('orders', 'placed'),
          title: 'Order placed',
          metadata: [
            {
              '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.CodeRefAnnotation',
              repo: 'storefront',
              path: 'apps/storefront/lib/storefront/orders/aggregate.ex',
              symbol: 'Storefront.Orders.Aggregate',
              ref: 'a1b2c3d4e5f6',
            },
          ],
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Implemented in')).toBeDefined();
    expect(screen.getByText('storefront')).toBeDefined();
    expect(screen.getByText('aggregate.ex')).toBeDefined();
    expect(screen.getByText('apps/storefront/lib/storefront/orders/')).toBeDefined();
    expect(screen.getByText('Storefront.Orders.Aggregate')).toBeDefined();
    expect(screen.getByText('a1b2c3d')).toBeDefined();
  });

  it('flags a code ref annotation missing its path', () => {
    const model = buildModel([
      {
        event: {
          id: mkId('orders', 'placed'),
          title: 'Order placed',
          metadata: [
            { '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.CodeRefAnnotation', repo: 'storefront' },
          ],
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText(/CODE_REF_INCOMPLETE/)).toBeDefined();
  });

  it('renders an unmapped annotation as a labelled key/value list', () => {
    const model = buildModel([
      {
        event: {
          id: mkId('orders', 'placed'),
          title: 'Order placed',
          metadata: [
            {
              '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.SharedConsumersAnnotation',
              doc: 'ledger and refunds',
            },
          ],
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Shared consumers')).toBeDefined();
    expect(screen.getByText('ledger and refunds')).toBeDefined();
  });

  it('renders a link annotation with a safe href', () => {
    const model = buildModel([
      {
        event: {
          id: mkId('orders', 'placed'),
          title: 'Order placed',
          metadata: [
            {
              '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.LinkAnnotation',
              url: 'https://example.com/docs',
              title: 'Docs',
            },
          ],
        },
      },
    ] as never);
    const event = mustFind(model.entities.find((e) => e.kind === 'event'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    const link = screen.getByText('Docs').closest('a');
    expect(link).toBeDefined();
    expect(link?.getAttribute('href')).toBe('https://example.com/docs');
  });

  it('renders the tracker items table', () => {
    const model = buildModel([
      {
        tracker: {
          id: mkId('orders', 'work'),
          title: 'Order work',
          items: [{ subject: { id: { slug: 'placed' } }, status: 'TRACK_STATUS_DONE', doc: 'shipped it' }],
        },
      },
    ] as never);
    const tracker = mustFind(model.entities.find((e) => e.kind === 'tracker'));
    render(<Inspector model={model} entity={tracker} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Items (1)')).toBeDefined();
    expect(screen.getByText('placed')).toBeDefined();
    expect(screen.getByText('shipped it')).toBeDefined();
  });
});

// The validator's findings were computed server-side and thrown away by the
// UI: a model the server rejects rendered exactly like a clean one.
describe('Inspector: validation findings', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  const rmModel = () =>
    buildModel([
      {
        readModel: {
          id: mkId('registry', 'changesets-awaiting-review'),
          title: 'Changesets awaiting review',
          sourceEvents: [
            { event: { id: mkId('registry', 'changeset.submitted') } },
            { event: { id: mkId('registry', 'changeset.rejected') } },
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

  it('shows the code, field, rule title, and message of a finding on the selected entity', () => {
    const model = rmModel();
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(
      <Inspector model={model} entity={rm} issues={buildIssueIndex([finding])} onClose={vi.fn()} onSelect={vi.fn()} />,
    );
    expect(screen.getByText('Validation (1)')).toBeDefined();
    expect(screen.getByText('RM_EVENT_NOT_PROJECTED')).toBeDefined();
    expect(screen.getByText('source_events')).toBeDefined();
    expect(screen.getByText('Read model declares an event nothing projects')).toBeDefined();
    expect(screen.getByText(/no in-model read-model slice projects it/)).toBeDefined();
  });

  it('shows no validation section for an entity with no findings', () => {
    const model = rmModel();
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(<Inspector model={model} entity={rm} issues={buildIssueIndex([])} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.queryByText(/^Validation/)).toBeNull();
  });

  it('says validation did not run rather than rendering the clean-model silence', () => {
    const model = rmModel();
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(
      <Inspector
        model={model}
        entity={rm}
        issues={buildIssueIndex(undefined, 'validate: 503')}
        onClose={vi.fn()}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByText(/unavailable \(validate: 503\)/)).toBeDefined();
  });
});
