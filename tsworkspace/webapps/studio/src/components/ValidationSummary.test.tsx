// Covers the two pieces of IssueIndex that had no renderer until now:
// the per-severity totals, and findings whose subject names no entity.
// Also covers the board badge, which is severity-driven and rule-agnostic
// on purpose (see IssueBadge.tsx).

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { IssueBadge } from '@/components/IssueBadge';
import { IssuesProvider, useIssues } from '@/components/IssuesProvider';
import { buildIssueIndex, EMPTY_ISSUE_INDEX } from '@/lib/issues';
import { ValidationSummary } from './ValidationSummary';

afterEach(() => {
  cleanup();
});

const attached = (severity: string, code: string, slug = 'changesets-awaiting-review') => ({
  severity,
  code,
  message: `${code} happened`,
  subject: { kind: 'ENTITY_KIND_READ_MODEL', id: { namespace: 'registry', slug, version: '1' } },
  subjectField: 'source_events',
});

const modelWide = (severity: string, code: string) => ({ severity, code, message: `${code} happened` });

const rm = {
  kind: 'readModel' as const,
  id: { namespace: 'registry', slug: 'changesets-awaiting-review', version: '1' },
};

describe('ValidationSummary', () => {
  const controls = { open: false, panelId: 'validation-findings', onToggle: () => undefined };

  it('shows a total per severity that is present', () => {
    const idx = buildIssueIndex([
      attached('SEVERITY_ERROR', 'DANGLING_REF'),
      attached('SEVERITY_WARNING', 'RM_EVENT_NOT_PROJECTED'),
      attached('SEVERITY_WARNING', 'ORPHAN_DOC_EMPTY'),
    ]);
    render(<ValidationSummary issues={idx} {...controls} />);
    expect(screen.getByText('1', { selector: '[data-severity="error"]' })).toBeDefined();
    expect(screen.getByText('2', { selector: '[data-severity="warning"]' })).toBeDefined();
    expect(document.querySelector('[data-severity="info"]')).toBeNull();
  });

  it('renders nothing when the model is clean', () => {
    const { container } = render(<ValidationSummary issues={buildIssueIndex([])} {...controls} />);
    expect(container.firstChild).toBeNull();
  });

  it('requests the drawer for attached-only findings and reflects controlled expanded state', () => {
    const idx = buildIssueIndex([attached('SEVERITY_ERROR', 'DANGLING_REF')]);
    const onToggle = vi.fn();
    const triggerRef = createRef<HTMLButtonElement>();
    const { rerender } = render(
      <ValidationSummary issues={idx} {...controls} onToggle={onToggle} triggerRef={triggerRef} />,
    );
    const trigger = screen.getByRole('button', { name: 'Validation findings' });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(triggerRef.current).toBe(trigger);
    fireEvent.click(trigger);
    expect(onToggle).toHaveBeenCalledOnce();
    expect(screen.queryByText('DANGLING_REF happened')).toBeNull();
    rerender(<ValidationSummary issues={idx} {...controls} open onToggle={onToggle} triggerRef={triggerRef} />);
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    expect(trigger.getAttribute('aria-controls')).toBe('validation-findings');
    fireEvent.click(trigger);
    expect(onToggle).toHaveBeenCalledTimes(2);
  });

  it('says validation is unavailable rather than showing a clean model', () => {
    render(<ValidationSummary issues={buildIssueIndex(undefined, 'validate: 503')} {...controls} />);
    expect(screen.getByText('validation unavailable')).toBeDefined();
  });

  it('counts a model-wide finding in the totals even though no entity carries it', () => {
    const idx = buildIssueIndex([modelWide('SEVERITY_ERROR', 'CROSS_MODEL_BOUNDARY_VIOLATION')]);
    render(<ValidationSummary issues={idx} {...controls} />);
    expect(screen.getByText('1', { selector: '[data-severity="error"]' })).toBeDefined();
    expect(screen.getByText('1 model-wide')).toBeDefined();
  });
});

describe('IssueBadge', () => {
  it('takes its severity from the worst finding, not the first reported', () => {
    const idx = buildIssueIndex([
      attached('SEVERITY_INFO', 'A_INFO'),
      attached('SEVERITY_ERROR', 'Z_ERROR'),
      attached('SEVERITY_WARNING', 'M_WARN'),
    ]);
    render(<IssueBadge issues={idx.for(rm)} />);
    const badge = screen.getByText('3', { selector: '[data-severity="error"]' });
    expect(badge).toBeDefined();
    expect(badge.getAttribute('title')).toContain('ERROR Z_ERROR');
    expect(badge.getAttribute('title')).toContain('INFO A_INFO');
  });

  it('renders nothing for an entity with no findings', () => {
    const { container } = render(<IssueBadge issues={[]} />);
    expect(container.firstChild).toBeNull();
  });
});

describe('IssuesProvider', () => {
  function Probe() {
    const issues = useIssues();
    return <span data-testid="probe">{`${issues.all.length}|${issues.error ?? 'no-error'}`}</span>;
  }

  // A node rendered outside a provider must report "no findings", never
  // "validated and clean"; EMPTY_ISSUE_INDEX carries no error either way.
  it('defaults to the empty index outside a provider', () => {
    render(<Probe />);
    expect(screen.getByTestId('probe').textContent).toBe('0|no-error');
    expect(EMPTY_ISSUE_INDEX.error).toBeUndefined();
  });

  it('hands the index to descendants', () => {
    render(
      <IssuesProvider issues={buildIssueIndex([attached('SEVERITY_ERROR', 'DANGLING_REF')])}>
        <Probe />
      </IssuesProvider>,
    );
    expect(screen.getByTestId('probe').textContent).toBe('1|no-error');
  });
});
