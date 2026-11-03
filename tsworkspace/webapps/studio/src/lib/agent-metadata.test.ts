import { describe, expect, it } from 'vitest';
import { buildModel } from '@/lib/model';
import {
  presentEntityRef,
  projectAgentAnnotation,
  projectBadgesAnnotation,
  projectLifecycleAnnotation,
  projectResourceGroupsAnnotation,
} from '@/lib/agent-metadata';

const mkId = (ns: string, slug: string, version = '2') => ({ namespace: ns, slug, version });

describe('agent-metadata projection', () => {
  it.each([undefined, '0', '-1', '18446744073709551616', 9007199254740992])(
    'rejects a resource reference without an exact uint64 version: %s',
    (version) => {
      const target = presentEntityRef(buildModel([]), {
        kind: 'ENTITY_KIND_COMPONENT',
        id: { namespace: 'agents', slug: 'worker', version },
      });
      expect(target.state).toBe('malformed');
    },
  );

  it('projects partial agent model and ordered tools without loss', () => {
    const view = projectAgentAnnotation({
      model: { provider: { value: 'Acme' }, version: { value: 'v1.0.0-beta' } },
      tools: [
        {
          name: { value: 'lookup' },
          type: { value: 'http' },
          description: { value: 'Read-only lookup' },
          icon: { value: 'search' },
          url: { value: 'https://example.com/docs/lookup' },
        },
        { name: { value: 'lookup' }, type: { value: 'sql' } },
      ],
    });
    expect(view.model?.provider).toBe('Acme');
    expect(view.model?.version).toBe('v1.0.0-beta');
    expect(view.tools.map((t) => `${t.name}/${t.type}`)).toEqual(['lookup/http', 'lookup/sql']);
    expect(view.tools[0]?.description).toBe('Read-only lookup');
    expect(view.issues).toHaveLength(0);
  });

  it('marks presence-only agent annotation', () => {
    const view = projectAgentAnnotation({});
    expect(view.presenceOnly).toBe(true);
  });

  it('surfaces malformed model instead of blank rows', () => {
    const view = projectAgentAnnotation({ model: { provider: { value: '' } } });
    expect(view.issues.some((i) => i.code === 'METADATA_BLANK' || i.code === 'METADATA_MALFORMED')).toBe(true);
  });

  it('projects lifecycle draft notice and exact deprecation date', () => {
    const view = projectLifecycleAnnotation({
      status: 'draft',
      draft: { title: { value: 'WIP' }, message: { value: 'Not ready' } },
      deprecatedOn: { value: '2024-02-29' },
    });
    expect(view.status).toBe('draft');
    expect(view.draft?.message).toBe('Not ready');
    expect(view.deprecatedOn).toBe('2024-02-29');
  });

  it('rejects malformed badge colors', () => {
    const view = projectBadgesAnnotation({
      badges: [{ content: { value: 'Beta' }, background_color: { value: 'red' }, text_color: { value: '#112233' } }],
    });
    expect(view.badges).toHaveLength(0);
    expect(view.issues.length).toBeGreaterThan(0);
  });

  it.each(['false', 0, 1, {}])('reports a malformed sidebar preference %j without coercion', (sidebar) => {
    const view = projectResourceGroupsAnnotation(
      { groups: [{ groupId: { value: 'd50199dd-2adb-4de0-854a-6df7f3c6c11a' }, sidebar }] },
      buildModel([]),
    );
    expect(view.groups[0]?.sidebar).toBeUndefined();
    expect(view.issues).toContainEqual({ code: 'METADATA_MALFORMED', detail: 'Sidebar preference is malformed' });
  });

  it.each([1, 500])('retains a valid display limit %j', (value) => {
    const view = projectResourceGroupsAnnotation(
      { groups: [{ groupId: { value: 'd50199dd-2adb-4de0-854a-6df7f3c6c11a' }, displayLimit: { value } }] },
      buildModel([]),
    );
    expect(view.groups[0]?.displayLimit).toBe(value);
    expect(view.issues).toHaveLength(0);
  });

  it.each([true, '1', 0, 501, 1.5])('reports a malformed display limit %j without coercion', (value) => {
    const view = projectResourceGroupsAnnotation(
      { groups: [{ groupId: { value: 'd50199dd-2adb-4de0-854a-6df7f3c6c11a' }, displayLimit: { value } }] },
      buildModel([]),
    );
    expect(view.groups[0]?.displayLimit).toBeUndefined();
    expect(view.issues).toContainEqual({ code: 'METADATA_MALFORMED', detail: 'Display limit is malformed' });
  });

  it('retains every group item despite display_limit', () => {
    const model = buildModel([
      { component: { id: mkId('agents', 'worker'), title: 'Worker' } },
      { component: { id: mkId('tools', 'catalog'), title: 'Catalog' } },
    ] as never);
    const view = projectResourceGroupsAnnotation(
      {
        groups: [
          {
            group_id: { value: 'group-g' },
            title: { value: 'Related' },
            display_limit: { value: 1 },
            items: [
              {
                connection_id: { value: 'item-i' },
                target: { kind: 'ENTITY_KIND_COMPONENT', id: mkId('tools', 'catalog') },
              },
              {
                connection_id: { value: 'item-j' },
                target: { kind: 'ENTITY_KIND_COMPONENT', id: mkId('tools', 'catalog') },
              },
            ],
          },
        ],
      },
      model,
    );
    expect(view.groups[0]?.items).toHaveLength(2);
    expect(view.groups[0]?.displayLimit).toBe(1);
    expect(view.groups[0]?.items[0]?.target.state).toBe('loaded');
    expect(view.groups[0]?.items[1]?.target.state).toBe('loaded');
  });

  it('shows unavailable targets without naming hidden entities', () => {
    const model = buildModel([{ component: { id: mkId('agents', 'worker'), title: 'Worker' } }] as never);
    const view = projectResourceGroupsAnnotation(
      {
        groups: [
          {
            group_id: { value: 'g1' },
            items: [
              {
                connection_id: { value: 'hidden-item' },
                target: { kind: 'ENTITY_KIND_COMPONENT', id: mkId('secret', 'x') },
              },
            ],
          },
        ],
      },
      model,
      { unavailableItemIds: new Set(['hidden-item']) },
    );
    expect(view.groups[0]?.items[0]?.target).toEqual({ state: 'unavailable' });
  });

  it('labels not-loaded refs with exact version keys', () => {
    const model = buildModel([]);
    const target = presentEntityRef(model, {
      kind: 'ENTITY_KIND_COMPONENT',
      id: mkId('tools', 'catalog', '7'),
    });
    expect(target).toEqual({
      state: 'not_loaded',
      label: 'component:tools/catalog@7',
    });
  });
});
