// Covers the node renderers StickyNode.branch.test.tsx doesn't touch: the
// StickyNode wrapper itself (orphan/pending/multi-issuer/conflict borders,
// ports), StickyCardBody's remaining variants (synthetic, roads, instance/
// moment, boundary sources), and the other board node components
// (SliceHeaderNode, GapHeaderNode, EventModelHeaderNode, StoryboardHeaderNode,
// BandNode, LaneNode, ContextHeaderNode).

import { cleanup, render, screen } from '@testing-library/react';
import { ReactFlowProvider } from '@xyflow/react';
import type { ComponentProps } from 'react';
import { afterEach, describe, expect, it } from 'vitest';

import { IssuesProvider } from '@/components/IssuesProvider';
import { buildIssueIndex } from '@/lib/issues';
import type { Entity } from '@/lib/model';
import { buildModel } from '@/lib/model';
import {
  BandNode,
  ContextHeaderNode,
  EventModelHeaderNode,
  GapHeaderNode,
  LaneNode,
  SliceHeaderNode,
  StickyCardBody,
  StickyNode,
  StoryboardHeaderNode,
} from './StickyNode';

// NodeProps requires many fields (width, height, dragging, zIndex, type, id,
// ...) that these render-only tests don't exercise. Rather than spreading a
// `{} as never` fragment into JSX (accepted at runtime by vitest/esbuild but
// rejected by tsc with TS2698 "Spread types may only be created from object
// types"), each test builds its data/selected/etc. props as a plain object
// and casts the whole thing to the target component's props type via this
// helper, which both type-checkers accept.
function asProps<T>(props: Record<string, unknown>): T {
  return props as unknown as T;
}

type StickyNodeProps = ComponentProps<typeof StickyNode>;
type SliceHeaderNodeProps = ComponentProps<typeof SliceHeaderNode>;
type GapHeaderNodeProps = ComponentProps<typeof GapHeaderNode>;
type EventModelHeaderNodeProps = ComponentProps<typeof EventModelHeaderNode>;
type StoryboardHeaderNodeProps = ComponentProps<typeof StoryboardHeaderNode>;
type BandNodeProps = ComponentProps<typeof BandNode>;
type LaneNodeProps = ComponentProps<typeof LaneNode>;
type ContextHeaderNodeProps = ComponentProps<typeof ContextHeaderNode>;

function eventEntity(overrides: Record<string, unknown> = {}): Entity {
  const model = buildModel([
    {
      event: {
        id: { namespace: 'shop', slug: 'order.placed', version: '1' },
        title: 'Order placed',
        ...overrides,
      },
    },
  ] as never);
  const found = model.entities.find((e) => e.kind === 'event');
  if (!found) throw new Error('fixture entity missing');
  return found;
}

function readModelEntity(overrides: Record<string, unknown> = {}): Entity {
  const model = buildModel([
    {
      readModel: {
        id: { namespace: 'shop', slug: 'order.summary', version: '1' },
        title: 'Order summary',
        ...overrides,
      },
    },
  ] as never);
  const found = model.entities.find((e) => e.kind === 'readModel');
  if (!found) throw new Error('fixture entity missing');
  return found;
}

function commandEntity(overrides: Record<string, unknown> = {}): Entity {
  const model = buildModel([
    {
      command: {
        id: { namespace: 'shop', slug: 'place.order', version: '1' },
        title: 'Place order',
        ...overrides,
      },
    },
  ] as never);
  const found = model.entities.find((e) => e.kind === 'command');
  if (!found) throw new Error('fixture entity missing');
  return found;
}

describe('StickyCardBody', () => {
  afterEach(cleanup);

  it('renders the synthetic implicit badge', () => {
    render(<StickyCardBody entity={eventEntity()} synthetic />);
    expect(screen.getAllByText('implicit').length).toBeGreaterThan(0);
  });

  it('renders roads with a count badge', () => {
    render(<StickyCardBody entity={eventEntity()} roads={[{ title: 'Cancel road', doc: 'timeout' }]} />);
    expect(screen.getAllByTitle(/opens: Cancel road/)[0].textContent).toBe('1');
  });

  it('renders the instance coordinate and moment tooltip', () => {
    render(<StickyCardBody entity={eventEntity()} instance="#2/4" moment={eventEntity()} />);
    expect(screen.getAllByText('#2/4').length).toBeGreaterThan(0);
  });

  it('renders the no-emitted-events pending badge', () => {
    render(<StickyCardBody entity={commandEntity()} noEmittedEvents />);
    expect(screen.getAllByText('pending').length).toBeGreaterThan(0);
  });

  it('renders the multiple-issuers error badge', () => {
    render(<StickyCardBody entity={commandEntity()} multipleIssuers />);
    expect(screen.getAllByText('multi-issuer').length).toBeGreaterThan(0);
  });

  it('renders the orphan badge', () => {
    render(
      <StickyCardBody
        entity={eventEntity({
          metadata: [
            { '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.OrphanAnnotation', doc: 'left dangling' },
          ],
        })}
      />,
    );
    expect(screen.getAllByText('orphan').length).toBeGreaterThan(0);
  });

  it('renders the external-source boundary indicator', () => {
    render(
      <StickyCardBody
        entity={readModelEntity({
          externalSource: { system: { id: { slug: 'billing' } }, descriptor: 'webhook' },
        })}
      />,
    );
    expect(screen.getAllByText('Order summary').length).toBeGreaterThan(0);
  });

  it('renders the cross-context translated boundary indicator', () => {
    render(
      <StickyCardBody
        entity={readModelEntity({
          sourceEvents: [{ id: { namespace: 'billing', slug: 'invoice.paid', version: '1' } }],
        })}
      />,
    );
    expect(screen.getAllByText('Order summary').length).toBeGreaterThan(0);
  });
});

describe('StickyNode', () => {
  afterEach(cleanup);

  it('renders the entity title', () => {
    render(
      <ReactFlowProvider>
        <StickyNode {...asProps<StickyNodeProps>({ data: { entity: eventEntity() }, selected: false })} />
      </ReactFlowProvider>,
    );
    expect(screen.getAllByText('Order placed').length).toBeGreaterThan(0);
  });

  it('applies the orphan dashed-border title', () => {
    const entity = eventEntity({
      metadata: [
        { '@type': 'type.googleapis.com/trogonatlas.eventmodel.v1alpha1.OrphanAnnotation', doc: 'left dangling' },
      ],
    });
    render(
      <ReactFlowProvider>
        <StickyNode {...asProps<StickyNodeProps>({ data: { entity }, selected: false })} />
      </ReactFlowProvider>,
    );
    expect(screen.getAllByTitle('Orphan: left dangling').length).toBeGreaterThan(0);
  });

  it('applies the multiple-issuers title over the other states', () => {
    render(
      <ReactFlowProvider>
        <StickyNode
          {...asProps<StickyNodeProps>({
            data: { entity: commandEntity(), multipleIssuers: true, noEmittedEvents: true },
            selected: false,
          })}
        />
      </ReactFlowProvider>,
    );
    expect(
      screen.getAllByTitle(
        'COMMAND_MULTIPLE_ISSUERS: this command is issued by more than one UI or processor; the trigger is ambiguous',
      ).length,
    ).toBeGreaterThan(0);
  });

  it('renders the conflict badge for a branch-conflict entity', () => {
    render(
      <ReactFlowProvider>
        <StickyNode
          {...asProps<StickyNodeProps>({ data: { entity: eventEntity(), branchStatus: 'conflict' }, selected: false })}
        />
      </ReactFlowProvider>,
    );
    expect(screen.getAllByText('conflict').length).toBeGreaterThan(0);
  });

  it('renders selected with the ring style applied without crashing', () => {
    render(
      <ReactFlowProvider>
        <StickyNode {...asProps<StickyNodeProps>({ data: { entity: eventEntity() }, selected: true })} />
      </ReactFlowProvider>,
    );
    expect(screen.getAllByText('Order placed').length).toBeGreaterThan(0);
  });
});

describe('SliceHeaderNode', () => {
  afterEach(cleanup);

  it('renders the slice kind label and title', () => {
    render(
      <SliceHeaderNode
        {...asProps<SliceHeaderNodeProps>({
          data: { sliceKind: 'commandSlice', entity: commandEntity() },
          selected: false,
        })}
      />,
    );
    expect(screen.getByText('Command Slice')).toBeDefined();
    expect(screen.getAllByText('Place order').length).toBeGreaterThan(0);
  });

  it('renders scenario count and status badges', () => {
    render(
      <SliceHeaderNode
        {...asProps<SliceHeaderNodeProps>({
          data: {
            sliceKind: 'readModelSlice',
            entity: readModelEntity({ scenarios: [{ id: 's1' }, { id: 's2' }] }),
            status: 'blocked',
            staleRefs: 2,
          },
          selected: true,
        })}
      />,
    );
    expect(screen.getByText('2 gwt')).toBeDefined();
    expect(screen.getByText('blocked')).toBeDefined();
    expect(screen.getByText('v↑2')).toBeDefined();
  });

  it('falls back to the raw slice kind label for an unknown kind', () => {
    render(
      <SliceHeaderNode
        {...asProps<SliceHeaderNodeProps>({
          data: { sliceKind: 'mysterySlice', entity: commandEntity() },
          selected: false,
        })}
      />,
    );
    expect(screen.getByText('mysterySlice')).toBeDefined();
  });
});

describe('GapHeaderNode', () => {
  afterEach(cleanup);

  it('renders the implicit-projection label and the read model/event summary', () => {
    render(
      <GapHeaderNode
        {...asProps<GapHeaderNodeProps>({
          data: { readModels: ['Order summary'], events: ['Order placed'] },
          selected: false,
        })}
      />,
    );
    expect(screen.getByText('Implicit projection: no slice yet')).toBeDefined();
    expect(screen.getByText('Order summary updated by Order placed')).toBeDefined();
  });
});

describe('EventModelHeaderNode', () => {
  afterEach(cleanup);

  it('renders the event model title and kind label', () => {
    render(
      <EventModelHeaderNode
        {...asProps<EventModelHeaderNodeProps>({ data: { entity: eventEntity() }, selected: false })}
      />,
    );
    expect(screen.getAllByText('Order placed').length).toBeGreaterThan(0);
    expect(screen.getByText('event model')).toBeDefined();
  });
});

describe('StoryboardHeaderNode', () => {
  afterEach(cleanup);

  it('renders the storyboard title without ways-in', () => {
    render(
      <StoryboardHeaderNode
        {...asProps<StoryboardHeaderNodeProps>({ data: { entity: eventEntity() }, selected: false })}
      />,
    );
    expect(screen.getAllByText('Order placed').length).toBeGreaterThan(0);
    expect(screen.getByText('storyboard')).toBeDefined();
  });

  it('renders a ways-in count badge', () => {
    render(
      <StoryboardHeaderNode
        {...asProps<StoryboardHeaderNodeProps>({
          data: { entity: eventEntity(), waysIn: [{ from: 'checkout', doc: 'timeout' }] },
          selected: false,
        })}
      />,
    );
    expect(screen.getByTitle(/from checkout/).textContent).toBe('1');
  });
});

describe('BandNode', () => {
  afterEach(cleanup);

  it('renders without crashing', () => {
    const { container } = render(<BandNode {...asProps<BandNodeProps>({ width: 100, height: 50 })} />);
    expect(container.firstChild).toBeTruthy();
  });
});

describe('LaneNode', () => {
  afterEach(cleanup);

  it('renders the lane label', () => {
    render(<LaneNode {...asProps<LaneNodeProps>({ data: { label: 'Checkout' }, width: 200, height: 40 })} />);
    expect(screen.getByText('Checkout')).toBeDefined();
  });

  it('renders a vector glyph for a lane with a named icon', () => {
    const { container } = render(
      <LaneNode {...asProps<LaneNodeProps>({ data: { label: 'Sally', glyph: 'persona' }, width: 200, height: 40 })} />,
    );
    expect(container.querySelector('svg')).toBeTruthy();
  });

  it('renders the stream id for a lane backed by an entity', () => {
    render(
      <LaneNode
        {...asProps<LaneNodeProps>({
          data: { label: 'Checkout', entity: eventEntity({ streamId: 'order-{orderId}' }), selected: true },
          width: 200,
          height: 40,
        })}
      />,
    );
    expect(screen.getByText('order-{orderId}')).toBeDefined();
  });
});

describe('ContextHeaderNode', () => {
  afterEach(cleanup);

  it('renders the namespace label only', () => {
    render(<ContextHeaderNode {...asProps<ContextHeaderNodeProps>({ data: { ns: 'shop' }, selected: false })} />);
    expect(screen.getByText('shop')).toBeDefined();
    expect(screen.getByText('bounded context')).toBeDefined();
  });

  it('renders an optional title alongside the namespace', () => {
    render(
      <ContextHeaderNode
        {...asProps<ContextHeaderNodeProps>({ data: { ns: 'shop', title: 'Shop context' }, selected: false })}
      />,
    );
    expect(screen.getByText('Shop context')).toBeDefined();
  });
});

// The board carries hand-written badges for two specific rule codes
// (see layout.ts). This badge is fed by the validator instead, so it covers
// every rule the server has, including ones written after this code.
describe('StickyCardBody: validator findings', () => {
  afterEach(() => {
    cleanup();
  });

  const finding = {
    severity: 'SEVERITY_WARNING',
    code: 'RM_EVENT_NOT_PROJECTED',
    message: 'declares an event no slice projects',
    subject: {
      kind: 'ENTITY_KIND_READ_MODEL',
      id: { namespace: 'shop', slug: 'order.summary', version: '1' },
    },
  };

  it('badges the card the finding names', () => {
    const entity = readModelEntity();
    render(
      <IssuesProvider issues={buildIssueIndex([finding])}>
        <StickyCardBody entity={entity} />
      </IssuesProvider>,
    );
    const badge = screen.getByText('1', { selector: '[data-severity="warning"]' });
    expect(badge.getAttribute('title')).toContain('RM_EVENT_NOT_PROJECTED');
  });

  it('leaves cards the finding does not name unbadged', () => {
    render(
      <IssuesProvider issues={buildIssueIndex([finding])}>
        <StickyCardBody entity={eventEntity()} />
      </IssuesProvider>,
    );
    expect(document.querySelector('[data-severity="warning"]')).toBeNull();
  });

  it('renders no badge when nothing validated the model', () => {
    render(<StickyCardBody entity={readModelEntity()} />);
    expect(document.querySelector('[data-severity]')).toBeNull();
  });
});
