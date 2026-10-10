import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ValidationInspector } from '@/components/ValidationInspector';
import { buildIssueIndex } from '@/lib/issues';
import { buildModel } from '@/lib/model';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const model = buildModel([]);

const attached = (severity: string, code: string) => ({
  severity,
  code,
  message: `${code} happened`,
  subject: { kind: 'ENTITY_KIND_READ_MODEL', id: { namespace: 'registry', slug: 'changeset-status', version: '1' } },
  subjectField: 'source_events',
});

describe('ValidationInspector', () => {
  it('copies a complete repair request for a model-wide finding without navigating', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const onSelect = vi.fn();
    const issues = buildIssueIndex([
      {
        severity: 'SEVERITY_ERROR',
        code: 'MODEL_INVALID',
        message: 'A storyboard must describe the intended order.',
      },
    ]);
    render(
      <ValidationInspector
        id="validation-findings"
        model={model}
        issues={issues}
        onClose={vi.fn()}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Copy fix prompt' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledOnce());
    const copied = String(writeText.mock.calls[0]?.[0] ?? '');
    expect(copied).toMatch(/fix.*validation/i);
    expect(copied).toContain('MODEL_INVALID');
    expect(copied).toContain('A storyboard must describe the intended order.');
    expect(copied).toMatch(/rerun validation/i);
    expect(onSelect).not.toHaveBeenCalled();
  });
  it('shows full attached-only findings in a named drawer', () => {
    const idx = buildIssueIndex([attached('SEVERITY_ERROR', 'DANGLING_REF')]);
    render(<ValidationInspector id="validation-findings" model={model} issues={idx} onClose={vi.fn()} />);
    expect(screen.getByRole('complementary', { name: 'Validation findings' }).id).toBe('validation-findings');
    expect(screen.getByText('DANGLING_REF happened')).toBeDefined();
    expect(screen.getByText('source_events')).toBeDefined();
    expect(screen.getByText('registry/changeset-status@1')).toBeDefined();
    expect(screen.getByText('error')).toBeDefined();
  });

  it('requests closing without owning shell dismissal or Escape handling', () => {
    const onClose = vi.fn();
    render(
      <ValidationInspector id="validation-findings" model={model} issues={buildIssueIndex([])} onClose={onClose} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Close validation findings' }));
    expect(onClose).toHaveBeenCalledOnce();
    expect(screen.getByRole('complementary', { name: 'Validation findings' })).toBeDefined();
  });

  it('filters attached and model-wide findings by severity', () => {
    const idx = buildIssueIndex([
      attached('SEVERITY_ERROR', 'DANGLING_REF'),
      attached('SEVERITY_WARNING', 'RM_EVENT_NOT_PROJECTED'),
      { severity: 'SEVERITY_INFO', code: 'MODEL_INFO', message: 'Model-wide information' },
    ]);
    render(<ValidationInspector id="validation-findings" model={model} issues={idx} onClose={vi.fn()} />);
    expect(screen.getByText('DANGLING_REF happened')).toBeDefined();
    expect(screen.getByText('RM_EVENT_NOT_PROJECTED happened')).toBeDefined();
    expect(screen.getByText('Model-wide information')).toBeDefined();
    const warnings = screen.getByRole('button', { name: 'Warnings (1)' });
    fireEvent.click(warnings);
    expect(warnings.getAttribute('aria-pressed')).toBe('true');
    expect(screen.queryByText('DANGLING_REF happened')).toBeNull();
    expect(screen.queryByText('Model-wide information')).toBeNull();
    expect(screen.getByText('RM_EVENT_NOT_PROJECTED happened')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'All (3)' }));
    expect(screen.getByText('DANGLING_REF happened')).toBeDefined();
    expect(screen.getByText('Model-wide information')).toBeDefined();
  });

  it('selects an attached entity using its full identity', () => {
    const idx = buildIssueIndex([attached('SEVERITY_ERROR', 'DANGLING_REF')]);
    const onSelect = vi.fn();
    render(
      <ValidationInspector id="validation-findings" model={model} issues={idx} onClose={vi.fn()} onSelect={onSelect} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Open read model registry/changeset-status@1' }));
    expect(onSelect).toHaveBeenCalledExactlyOnceWith({
      kind: 'readModel',
      id: { namespace: 'registry', slug: 'changeset-status', version: '1' },
    });
  });

  it('keeps unavailable subjects readable without offering an inert entity button', () => {
    const idx = buildIssueIndex([attached('SEVERITY_ERROR', 'DANGLING_REF')]);
    render(
      <ValidationInspector
        model={model}
        id="validation-findings"
        issues={idx}
        onClose={vi.fn()}
        onSelect={vi.fn()}
        canSelect={() => false}
      />,
    );
    expect(screen.getByText('registry/changeset-status@1')).toBeDefined();
    expect(screen.queryByRole('button', { name: /Open read model/ })).toBeNull();
  });

  it('updates an open drawer when validation becomes unavailable or findings are cleared', () => {
    const idx = buildIssueIndex([attached('SEVERITY_ERROR', 'DANGLING_REF')]);
    const onClose = vi.fn();
    const { rerender } = render(
      <ValidationInspector id="validation-findings" model={model} issues={idx} onClose={onClose} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Errors (1)' }));
    rerender(
      <ValidationInspector
        model={model}
        id="validation-findings"
        issues={buildIssueIndex(undefined, 'validate: 503')}
        onClose={onClose}
      />,
    );
    expect(screen.getByRole('status').textContent).toContain('Validation unavailable');
    expect(screen.getByText('validate: 503')).toBeDefined();
    expect(screen.queryByText('DANGLING_REF happened')).toBeNull();
    expect(screen.queryByText('No validation findings.')).toBeNull();
    rerender(
      <ValidationInspector id="validation-findings" model={model} issues={buildIssueIndex([])} onClose={onClose} />,
    );
    expect(screen.getByText('No validation findings.')).toBeDefined();
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByRole('button', { name: 'Close validation findings' })).toBeDefined();
  });

  it('shows an empty filter without hiding the remaining findings total', () => {
    const idx = buildIssueIndex([attached('SEVERITY_ERROR', 'DANGLING_REF')]);
    render(<ValidationInspector id="validation-findings" model={model} issues={idx} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Warnings (0)' }));
    expect(screen.getByText('No findings at this severity.')).toBeDefined();
    expect(screen.getByRole('button', { name: 'All (1)' })).toBeDefined();
  });
});
