// Render coverage for Inspector's main body: Inspector.test.tsx already
// covers safeLinkHref in isolation; this file exercises the component's
// section-by-section rendering (Timeline, Doc, Fields, Annotations,
// Scenarios, Slices membership, Tech, Problem space, Tracking, etc.) using
// realistic wire fixtures built through buildModel, following the fixture
// idiom in src/lib/model.test.ts (commandSliceFixture-style wire arrays).
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { type ComponentProps, useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/CopyContextButton', () => ({
  CopyContextButton: () => <button type="button">Copy context</button>,
}));

vi.mock('@/lib/useEntityHomes', () => ({
  useEntityHomes: () => ({ homes: new Map(), error: undefined, retry: vi.fn() }),
}));

import * as api from '@/lib/api';
import { buildIssueIndex } from '@/lib/issues';
import { buildModel, type Entity } from '@/lib/model';
import { Inspector, type InspectorTab } from './Inspector';

const mkId = (ns: string, slug: string) => ({ namespace: ns, slug, version: '1' });

function mustFind(e: Entity | undefined): Entity {
  if (!e) throw new Error('fixture entity missing');
  return e;
}

function commandSliceFixture(withScenarios = true): api.WireEntityEnvelope[] {
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
        scenarios: withScenarios
          ? [
              {
                id: 'happy-path',
                title: 'Order placed successfully',
                given: [{ event: { id: mkId(ns, 'rejected') }, payload: { reason: 'retry' } }],
                when: { command: { id: mkId(ns, 'place') }, payload: { total: 42 } },
                emit: { events: [{ event: { id: mkId(ns, 'placed') }, payload: { orderId: 'o-1' } }] },
              },
            ]
          : [],
      },
    },
  ] as never;
}

function baseModel() {
  return buildModel(commandSliceFixture());
}

function ManagedInspector({
  mode,
  ...props
}: Omit<ComponentProps<typeof Inspector>, 'tab' | 'onTabChange'> & { mode: 'controlled' | 'uncontrolled' }) {
  const [tab, setTab] = useState<InspectorTab>('details');
  return (
    <Inspector
      {...props}
      tab={mode === 'controlled' ? tab : undefined}
      onTabChange={mode === 'controlled' ? setTab : undefined}
    />
  );
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

  it.each([undefined, '11111111-1111-4111-8111-111111111111'] as const)(
    'keeps responsibility controls and lookups out of entity Details at %s',
    (asOf) => {
      const queries = [
        vi.spyOn(api, 'listSubjectAssignments'),
        vi.spyOn(api, 'listAssigneeResponsibilities'),
        vi.spyOn(api, 'listTeamMembers'),
        vi.spyOn(api, 'listPersonTeams'),
        vi.spyOn(api, 'getTeamContacts'),
      ];
      const model = buildModel([
        {
          team: {
            id: mkId('catalog', 'team'),
            title: 'Catalog team',
            contacts: [{ email: { value: 'team@example.test' } }],
          },
        },
      ] as never);
      const team = mustFind(model.entities[0]);
      render(<Inspector entity={team} model={model} asOf={asOf} onClose={vi.fn()} onSelect={vi.fn()} />);
      expect(screen.getByRole('tab', { name: 'Details', selected: true })).toBeDefined();
      expect(screen.queryByRole('heading', { name: 'Responsibilities' })).toBeNull();
      expect(screen.queryByLabelText('Assignment role')).toBeNull();
      expect(screen.queryByText(/Historical responsibility lookups are unavailable/)).toBeNull();
      for (const query of queries) expect(query).not.toHaveBeenCalled();
    },
  );

  it('renders descriptive storage resource details without an execution timeline', () => {
    const model = buildModel([
      {
        dataStore: {
          id: mkId('orders', 'archive'),
          title: 'Order archive',
          storeClass: { known: 'KNOWN_OBJECT_STORE' },
          authoritative: false,
          attachments: [
            { connectionId: { value: 'archive-attachment' }, readModel: { id: mkId('orders', 'summary') } },
          ],
        },
      },
    ]);
    const entity = mustFind(model.byKind.get('dataStore')?.[0]);
    render(<Inspector model={model} entity={entity} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('Storage declaration')).toBeDefined();
    expect(screen.getByText('No')).toBeDefined();
    expect(screen.getByText('archive-attachment')).toBeDefined();
    expect(screen.queryByText('Timeline')).toBeNull();
  });

  it('renders the doc section first when doc is present', () => {
    const model = baseModel();
    const event = mustFind(model.entities.find((e) => e.id.slug === 'placed'));
    render(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByText('The order was placed.')).toBeDefined();
    const details = screen.getByRole('tabpanel', { name: 'Details' });
    const headings = within(details).getAllByRole('heading');
    expect(headings[0]?.textContent).toBe('Doc');
    expect(headings[1]?.textContent).toBe('Timeline');
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

  it('shows slice scenarios in the second tab after Details', () => {
    const decisions = vi.spyOn(api, 'listSubjectDecisions');
    const documents = vi.spyOn(api, 'listSubjectDocuments');
    const sourceReview = vi.spyOn(api, 'inspectSource');
    const model = baseModel();
    const slice = mustFind(model.slices[0]?.entity);
    const { rerender } = render(<Inspector model={model} entity={slice} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['Details', 'Scenarios (1)', 'History']);
    expect(screen.getByRole('tab', { name: 'Details' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.queryByRole('heading', { name: 'Decision and Document inspection' })).toBeNull();
    expect(screen.queryByLabelText('Exact catalog subject')).toBeNull();
    expect(screen.queryByRole('heading', { name: 'Source integration review' })).toBeNull();
    expect(screen.queryByLabelText('Review records')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Inspect backend records' })).toBeNull();
    expect(decisions).not.toHaveBeenCalled();
    expect(documents).not.toHaveBeenCalled();
    expect(sourceReview).not.toHaveBeenCalled();
    expect(screen.queryByText('Order placed successfully')).toBeNull();
    fireEvent.click(screen.getByRole('tab', { name: 'Scenarios (1)' }));
    expect(screen.getByText('Order placed successfully')).toBeDefined();

    const event = mustFind(model.entities.find((entity) => entity.id.slug === 'placed'));
    rerender(<Inspector model={model} entity={event} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.queryByRole('tab', { name: /Scenarios/ })).toBeNull();
    expect(screen.getByRole('tab', { name: 'Details' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.queryByText('Order placed successfully')).toBeNull();
    expect(decisions).not.toHaveBeenCalled();
    expect(documents).not.toHaveBeenCalled();
    expect(sourceReview).not.toHaveBeenCalled();
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
    fireEvent.click(screen.getByRole('tab', { name: 'Scenarios (1)' }));
    expect(screen.getByText('Given')).toBeDefined();
    expect(screen.getByText('nothing')).toBeDefined();
  });

  it('shows the Scenarios tab with an empty state when a slice has no scenarios', () => {
    const model = buildModel(commandSliceFixture(false));
    const entity = mustFind(model.slices[0]?.entity);
    render(<Inspector model={model} entity={entity} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['Details', 'Scenarios (0)', 'History']);
    fireEvent.click(screen.getByRole('tab', { name: 'Scenarios (0)' }));
    expect(screen.getByText('This slice has no scenarios.')).toBeDefined();
  });

  it.each(['controlled', 'uncontrolled'] as const)(
    'keeps Scenarios selected when scenarios disappear and return in %s usage',
    (mode) => {
      const model = baseModel();
      const emptyModel = buildModel(commandSliceFixture(false));
      const props = { mode, model, entity: mustFind(model.slices[0]?.entity), onClose: vi.fn(), onSelect: vi.fn() };
      const { rerender } = render(<ManagedInspector {...props} />);
      fireEvent.click(screen.getByRole('tab', { name: 'Scenarios (1)' }));
      expect(screen.getByRole('tab', { name: 'Scenarios (1)', selected: true })).toBeDefined();

      rerender(<ManagedInspector {...props} model={emptyModel} entity={mustFind(emptyModel.slices[0]?.entity)} />);
      expect(screen.getByRole('tab', { name: 'Scenarios (0)', selected: true })).toBeDefined();
      expect(screen.getByText('This slice has no scenarios.')).toBeDefined();

      rerender(<ManagedInspector {...props} />);
      expect(screen.getByRole('tab', { name: 'Scenarios (1)', selected: true })).toBeDefined();
      expect(screen.getByText('Order placed successfully')).toBeDefined();
    },
  );

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
          scenarios: [
            {
              id: 'projects',
              title: 'Feed projects',
              when: [{ event: { id: mkId(ns, 'placed') } }],
              // oxlint-disable-next-line unicorn/no-thenable -- ReadModelScenario names its protobuf outcome "then".
              then: { readModel: { id: mkId(ns, 'orders-feed') } },
            },
          ],
        },
      },
    ] as never);
    const entity = mustFind(model.slices[0]?.entity);
    render(<Inspector model={model} entity={entity} onClose={vi.fn()} onSelect={vi.fn()} />);
    fireEvent.click(screen.getByRole('tab', { name: 'Scenarios (1)' }));
    expect(screen.getByText('Feed projects')).toBeDefined();
    expect(screen.getByText('When')).toBeDefined();
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
          doc: 'Everything waiting for someone to look at it.',
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

  it('opens on the Details tab by default, with findings hidden and entity sections visible', () => {
    const model = rmModel();
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(
      <Inspector model={model} entity={rm} issues={buildIssueIndex([finding])} onClose={vi.fn()} onSelect={vi.fn()} />,
    );
    expect(screen.getByRole('tab', { name: 'Details', selected: true })).toBeDefined();
    expect(screen.getByText('Everything waiting for someone to look at it.')).toBeDefined();
    expect(screen.queryByText('RM_EVENT_NOT_PROJECTED')).toBeNull();
    expect(screen.queryByText(finding.message)).toBeNull();
  });

  it('shows a count pill on the Validation tab when the entity has findings', () => {
    const model = rmModel();
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(
      <Inspector model={model} entity={rm} issues={buildIssueIndex([finding])} onClose={vi.fn()} onSelect={vi.fn()} />,
    );
    const validationTab = screen.getByRole('tab', { name: /Validation/ });
    expect(within(validationTab).getByText('1')).toBeDefined();
  });

  it("reveals the finding's code, field, rule title, and message when the Validation tab is clicked", () => {
    const model = rmModel();
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(
      <Inspector model={model} entity={rm} issues={buildIssueIndex([finding])} onClose={vi.fn()} onSelect={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
    expect(screen.getByRole('tab', { name: /Validation/, selected: true })).toBeDefined();
    expect(screen.getByText('RM_EVENT_NOT_PROJECTED')).toBeDefined();
    expect(screen.getByText('source_events')).toBeDefined();
    expect(screen.getByText('Read model declares an event nothing projects')).toBeDefined();
    expect(screen.getByText(/no in-model read-model slice projects it/)).toBeDefined();
  });

  it.each(['controlled', 'uncontrolled'] as const)(
    'hides empty validation and returns to Details in %s usage',
    (mode) => {
      const model = rmModel();
      const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
      const props = {
        model,
        entity: rm,
        tab: mode === 'controlled' ? ('validation' as const) : undefined,
        onClose: vi.fn(),
        onSelect: vi.fn(),
      };
      const { rerender } = render(<Inspector {...props} issues={buildIssueIndex([])} />);
      expect(screen.queryByRole('tab', { name: /Validation/ })).toBeNull();
      expect(screen.queryByRole('tabpanel', { name: /Validation/ })).toBeNull();
      expect(screen.getByRole('tab', { name: 'Details', selected: true })).toBeDefined();

      rerender(<Inspector {...props} issues={buildIssueIndex([finding])} />);
      if (mode === 'uncontrolled') fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
      expect(screen.getByRole('tab', { name: /Validation/, selected: true })).toBeDefined();
      expect(screen.getByText('RM_EVENT_NOT_PROJECTED')).toBeDefined();

      rerender(<Inspector {...props} issues={buildIssueIndex([])} />);
      expect(screen.queryByRole('tab', { name: /Validation/ })).toBeNull();
      expect(screen.queryByRole('tabpanel', { name: /Validation/ })).toBeNull();
      expect(screen.getByRole('tab', { name: 'Details', selected: true })).toBeDefined();
      expect(screen.getByText('Everything waiting for someone to look at it.')).toBeDefined();
      expect(screen.queryByText('RM_EVENT_NOT_PROJECTED')).toBeNull();
    },
  );

  it.each(['controlled', 'uncontrolled'] as const)(
    'keeps Details selected when validation findings disappear and return in %s usage',
    (mode) => {
      const model = rmModel();
      const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
      const props = { mode, model, entity: rm, onClose: vi.fn(), onSelect: vi.fn() };
      const { rerender } = render(<ManagedInspector {...props} issues={buildIssueIndex([finding])} />);
      fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
      expect(screen.getByRole('tab', { name: /Validation/, selected: true })).toBeDefined();

      rerender(<ManagedInspector {...props} issues={buildIssueIndex([])} />);
      expect(screen.queryByRole('tab', { name: /Validation/ })).toBeNull();
      expect(screen.getByRole('tab', { name: 'Details', selected: true })).toBeDefined();

      rerender(<ManagedInspector {...props} issues={buildIssueIndex([finding])} />);
      expect(screen.queryByRole('tab', { name: 'Details', selected: true })).not.toBeNull();
      expect(screen.queryByText('RM_EVENT_NOT_PROJECTED')).toBeNull();
      fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
      expect(screen.getByRole('tab', { name: /Validation/, selected: true })).toBeDefined();
    },
  );

  it('marks the Validation tab and panel when validation did not run, instead of rendering the clean-model silence', () => {
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
    expect(screen.getByTitle('Validation unavailable')).toBeDefined();
    fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
    expect(screen.getByText(/unavailable \(validate: 503\)/)).toBeDefined();
  });

  it('renders the Validation panel directly when the tab prop is controlled', () => {
    const model = rmModel();
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    render(
      <Inspector
        model={model}
        entity={rm}
        issues={buildIssueIndex([finding])}
        tab="validation"
        onClose={vi.fn()}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByRole('tab', { name: /Validation/, selected: true })).toBeDefined();
    expect(screen.getByText('RM_EVENT_NOT_PROJECTED')).toBeDefined();
  });

  it('calls onTabChange with the clicked tab', () => {
    const model = rmModel();
    const rm = mustFind(model.entities.find((e) => e.kind === 'readModel'));
    const onTabChange = vi.fn();
    render(
      <Inspector
        model={model}
        entity={rm}
        issues={buildIssueIndex([finding])}
        onTabChange={onTabChange}
        onClose={vi.fn()}
        onSelect={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('tab', { name: /Validation/ }));
    expect(onTabChange).toHaveBeenCalledWith('validation');
  });

  it('renders agent and shared metadata annotations from decoded fixtures', () => {
    const model = buildModel([
      { component: { id: mkId('agents', 'worker'), title: 'Worker agent' } },
      { component: { id: mkId('tools', 'search'), title: 'Search tool target' } },
      {
        component: {
          id: mkId('agents', 'router'),
          title: 'Router agent',
          metadata: [
            {
              '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation',
              model: { provider: { value: 'Example' }, version: { value: '1.0.0' } },
              tools: [{ name: { value: 'search' }, type: { value: 'builtin' } }],
            },
            {
              '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation',
              status: 'draft',
              draft: { message: { value: 'Still wiring tools' } },
            },
            {
              '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.BadgesAnnotation',
              badges: [
                {
                  content: { value: 'Preview' },
                  backgroundColor: { value: '#EEFF00' },
                  textColor: { value: '#000000' },
                },
              ],
            },
            {
              '@type': 'type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation',
              groups: [
                {
                  groupId: { value: 'grp-1' },
                  displayLimit: { value: 1 },
                  items: [
                    {
                      connectionId: { value: 'sel-a' },
                      target: { kind: 'ENTITY_KIND_COMPONENT', id: mkId('tools', 'search') },
                    },
                    {
                      connectionId: { value: 'sel-b' },
                      target: { kind: 'ENTITY_KIND_COMPONENT', id: mkId('tools', 'search') },
                    },
                  ],
                },
              ],
            },
          ],
        },
      },
    ] as never);
    const agent = mustFind(model.entities.find((e) => e.id.slug === 'router'));
    render(<Inspector model={model} entity={agent} onClose={vi.fn()} onSelect={vi.fn()} />);
    expect(screen.getByTestId('agent-annotation')).toBeDefined();
    expect(screen.getByText('Example')).toBeDefined();
    expect(screen.getByText('Still wiring tools')).toBeDefined();
    expect(screen.getByText('Preview')).toBeDefined();
    expect(screen.getByTestId('group-items-grp-1').querySelectorAll('li')).toHaveLength(2);
  });
});
