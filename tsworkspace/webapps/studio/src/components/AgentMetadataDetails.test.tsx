import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Annotation } from '@/lib/model';
import {
  AgentAnnotationDetails,
  BadgesAnnotationDetails,
  LifecycleAnnotationDetails,
  ResourceGroupsAnnotationDetails,
} from './AgentMetadataDetails';
import { buildModel } from '@/lib/model';

const mkId = (ns: string, slug: string, version = '1') => ({ namespace: ns, slug, version });

function ann(type: string, data: Record<string, unknown>): Annotation {
  return { type, data };
}

describe('AgentMetadataDetails', () => {
  afterEach(() => cleanup());

  it('keeps a gateway decode failure visible instead of declaring an empty agent', () => {
    render(
      <AgentAnnotationDetails annotation={ann('AgentAnnotation', { '@decodeError': 'Malformed native annotation' })} />,
    );
    expect(screen.getByTestId('metadata-projection-issue').textContent).toContain('Malformed native annotation');
    expect(screen.queryByText('Declared agent (no model or tools)')).toBeNull();
  });

  it.each(['http://example.com/docs', 'https://user:secret@example.com/docs', '/relative-docs'])(
    'blocks a metadata documentation URL outside its HTTPS-only contract: %s',
    (url) => {
      const { container } = render(
        <AgentAnnotationDetails
          annotation={ann('AgentAnnotation', {
            tools: [{ name: { value: 'lookup' }, type: { value: 'http' }, url: { value: url } }],
          })}
        />,
      );
      expect(screen.queryByRole('link')).toBeNull();
      expect(container.innerHTML).not.toContain('user:secret');
    },
  );

  it('reports a malformed tools collection instead of a presence-only declaration', () => {
    render(<AgentAnnotationDetails annotation={ann('AgentAnnotation', { tools: 'invalid' })} />);
    expect(screen.getByTestId('metadata-projection-issue')).toBeDefined();
    expect(screen.queryByText('Declared agent (no model or tools)')).toBeNull();
  });

  it('reports a malformed badges collection instead of empty metadata', () => {
    render(<BadgesAnnotationDetails annotation={ann('BadgesAnnotation', { badges: {} })} />);
    expect(screen.getByTestId('metadata-projection-issue')).toBeDefined();
  });

  it('reports a malformed groups collection instead of empty metadata', () => {
    render(
      <ResourceGroupsAnnotationDetails
        annotation={ann('ResourceGroupsAnnotation', { groups: true })}
        model={buildModel([])}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByTestId('metadata-projection-issue')).toBeDefined();
  });

  it('renders agent model, tool order and inert documentation links', () => {
    render(
      <AgentAnnotationDetails
        annotation={ann('AgentAnnotation', {
          model: { name: { value: 'helper' }, version: { value: '2026-01-01' } },
          tools: [
            { name: { value: 'alpha' }, type: { value: 'builtin' } },
            {
              name: { value: 'beta' },
              type: { value: 'http' },
              url: { value: 'https://example.com/beta' },
            },
          ],
        })}
      />,
    );
    expect(screen.getByTestId('agent-annotation')).toBeDefined();
    expect(screen.getByText('helper')).toBeDefined();
    expect(screen.getByText('2026-01-01')).toBeDefined();
    const tools = screen.getByTestId('agent-tools');
    expect(tools.textContent?.indexOf('alpha')).toBeLessThan(tools.textContent?.indexOf('beta') ?? 0);
    const link = screen.getByRole('link', { name: /documentation/i });
    expect(link.getAttribute('href')).toBe('https://example.com/beta');
  });

  it('shows projection issues for malformed tool rows', () => {
    render(
      <AgentAnnotationDetails
        annotation={ann('AgentAnnotation', {
          tools: [{ name: { value: 'only-name' }, type: { value: '' } }],
        })}
      />,
    );
    expect(screen.getAllByTestId('metadata-projection-issue').length).toBeGreaterThan(0);
  });

  it('renders lifecycle draft and deprecated date', () => {
    render(
      <LifecycleAnnotationDetails
        annotation={ann('LifecycleAnnotation', {
          status: 'deprecated',
          doc: 'use v2',
          deprecatedOn: { value: '2026-12-31' },
        })}
      />,
    );
    expect(screen.getByText('deprecated')).toBeDefined();
    expect(screen.getByTestId('lifecycle-deprecated-on').textContent).toContain('2026-12-31');
  });

  it('removes only the selected badge when equal labels have different icons', () => {
    const badge = (content: string, icon: string) => ({
      content: { value: content },
      backgroundColor: { value: '#AABBCC' },
      textColor: { value: '#001122' },
      icon: { value: icon },
    });
    const search = badge('Beta', 'search');
    const info = badge('Beta', 'info');
    const stable = badge('Stable', 'check');
    const { rerender } = render(
      <BadgesAnnotationDetails annotation={ann('BadgesAnnotation', { badges: [search, info, stable] })} />,
    );
    expect(screen.getAllByText('Beta')).toHaveLength(2);
    rerender(<BadgesAnnotationDetails annotation={ann('BadgesAnnotation', { badges: [stable, info] })} />);
    expect(screen.getAllByText('Beta')).toHaveLength(1);
    expect(screen.queryByText('search')).toBeNull();
    expect(screen.getByTestId('badges-annotation').textContent?.indexOf('Stable')).toBeLessThan(
      screen.getByTestId('badges-annotation').textContent?.indexOf('Beta') ?? 0,
    );
  });

  it('renders badge colors as escaped text with safe styling', () => {
    render(
      <BadgesAnnotationDetails
        annotation={ann('BadgesAnnotation', {
          badges: [
            {
              content: { value: '<script>alert(1)</script>' },
              backgroundColor: { value: '#AABBCC' },
              textColor: { value: '#001122' },
            },
          ],
        })}
      />,
    );
    expect(screen.getByText('<script>alert(1)</script>')).toBeDefined();
  });

  it.each([true, false, undefined])('preserves the authored sidebar preference %s', (sidebar) => {
    render(
      <ResourceGroupsAnnotationDetails
        annotation={ann('ResourceGroupsAnnotation', {
          groups: [{ groupId: { value: 'd50199dd-2adb-4de0-854a-6df7f3c6c11a' }, sidebar }],
        })}
        model={buildModel([])}
        onSelect={vi.fn()}
      />,
    );
    if (sidebar === true) expect(screen.getByText('sidebar')).toBeDefined();
    else if (sidebar === false) expect(screen.getByText('not in sidebar')).toBeDefined();
    else {
      expect(screen.queryByText('sidebar')).toBeNull();
      expect(screen.queryByText('not in sidebar')).toBeNull();
    }
  });

  it('lists every resource group item and navigates loaded targets only', () => {
    const model = buildModel([
      { component: { id: mkId('agents', 'router'), title: 'Router' } },
      { component: { id: mkId('agents', 'worker'), title: 'Worker' } },
    ] as never);
    const onSelect = vi.fn();
    render(
      <ResourceGroupsAnnotationDetails
        annotation={ann('ResourceGroupsAnnotation', {
          groups: [
            {
              groupId: { value: 'g-main' },
              displayLimit: { value: 1 },
              items: [
                {
                  connectionId: { value: 'i-1' },
                  target: { kind: 'ENTITY_KIND_COMPONENT', id: mkId('agents', 'worker') },
                },
                {
                  connectionId: { value: 'i-2' },
                  target: { kind: 'ENTITY_KIND_COMPONENT', id: mkId('agents', 'worker', '3') },
                },
              ],
            },
          ],
        })}
        model={model}
        onSelect={onSelect}
      />,
    );
    expect(screen.getByTestId('group-items-g-main').querySelectorAll('li')).toHaveLength(2);
    fireEvent.click(screen.getByRole('button', { name: /component:agents\/worker@1/ }));
    expect(onSelect).toHaveBeenCalledWith('component:agents/worker@1');
    expect(screen.getByText(/component:agents\/worker@3 \(not loaded\)/)).toBeDefined();
  });

  it('shows unavailable without leaking hidden identity', () => {
    const model = buildModel([{ component: { id: mkId('agents', 'router'), title: 'Router' } }] as never);
    render(
      <ResourceGroupsAnnotationDetails
        annotation={ann('ResourceGroupsAnnotation', {
          groups: [
            {
              groupId: { value: 'g1' },
              items: [
                { connectionId: { value: 'redacted' }, target: { kind: 'ENTITY_KIND_COMPONENT', id: mkId('x', 'y') } },
              ],
            },
          ],
        })}
        model={model}
        onSelect={vi.fn()}
        unavailableItemIds={new Set(['redacted'])}
      />,
    );
    expect(screen.getByText('Unavailable')).toBeDefined();
    expect(screen.queryByText(/component:x\/y/)).toBeNull();
  });
});
