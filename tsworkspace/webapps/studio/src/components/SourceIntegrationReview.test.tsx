import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SourceIntegrationReview } from './SourceIntegrationReview';

const inspect = vi.hoisted(() => vi.fn());
vi.mock('@/lib/api', () => ({ inspectSource: inspect }));
vi.mock('@/lib/branch', () => ({ readBranchFromUrl: () => 'review' }));
beforeEach(() => {
  inspect.mockReset();
});
afterEach(cleanup);

function openRun() {
  fireEvent.change(screen.getByLabelText('Review records'), { target: { value: 'run' } });
  fireEvent.change(screen.getByLabelText('Exact registration ID'), { target: { value: 'registration' } });
  fireEvent.change(screen.getByLabelText('Exact run ID'), { target: { value: 'run' } });
  fireEvent.click(screen.getByText('Inspect backend records'));
}

describe('source review evidence boundaries', () => {
  it('retains unknown outcomes, incomplete evidence, exact revisions and inert pointers', async () => {
    inspect.mockResolvedValue({
      run: {
        id: 'run',
        revision: 'revision',
        status: 'SOURCE_RUN_STATUS_OUTCOME_UNKNOWN',
        recoveryRequired: true,
        detailsOmitted: true,
        incomplete: true,
        fidelity: { checkedCoverage: 'SOURCE_COVERAGE_PARTIAL', nativeReapplySupported: false },
        pointer: '<script>unsafe()</script>',
      },
    });
    const { container } = render(<SourceIntegrationReview namespace="models" />);
    openRun();
    await screen.findByText(/Recovery required/);
    expect(inspect).toHaveBeenCalledWith(
      'run',
      { registrationId: 'registration', runId: 'run' },
      expect.objectContaining({ branch: 'review', signal: expect.any(AbortSignal) }),
    );
    expect(screen.getByText(/Missing records do not prove absence/)).toBeTruthy();
    expect(screen.getByText(/Run summary omits details/)).toBeTruthy();
    expect(container.querySelector('script')).toBeNull();
    expect(container.textContent).toContain('<script>unsafe()</script>');
    fireEvent.change(screen.getByLabelText('Exact run ID'), { target: { value: 'other' } });
    expect(screen.queryByText(/Recovery required/)).toBeNull();
  });
  it('shows declared completeness separately from verified authority and never requests raw bytes', async () => {
    inspect.mockResolvedValue({
      snapshot: { declaredCoverage: 'SOURCE_COVERAGE_COMPLETE', bundleDigest: 'bytes-digest' },
    });
    render(<SourceIntegrationReview namespace="models" />);
    fireEvent.change(screen.getByLabelText('Review records'), { target: { value: 'snapshot' } });
    fireEvent.change(screen.getByLabelText('Exact registration ID'), { target: { value: 'registration' } });
    fireEvent.change(screen.getByLabelText('Exact snapshot ID'), { target: { value: 'snapshot' } });
    fireEvent.click(screen.getByText('Inspect backend records'));
    await screen.findByText(/does not verify completeness or authorize deletion/);
    expect(inspect.mock.calls[0][1]).toEqual({
      registrationId: 'registration',
      snapshotId: 'snapshot',
      includeBytes: false,
    });
    expect(screen.getByText(/Lossless recovery and native reapply remain unproven/)).toBeTruthy();
  });
  it('does not hydrate historical source state and aborts pending reads on unmount', async () => {
    const historical = render(<SourceIntegrationReview namespace="models" asOf="old" />);
    expect(screen.getByText(/Historical source hydration is unavailable/)).toBeTruthy();
    expect(inspect).not.toHaveBeenCalled();
    historical.unmount();
    inspect.mockImplementation(() => new Promise(() => {}));
    const live = render(<SourceIntegrationReview namespace="models" />);
    openRun();
    await waitFor(() => expect(inspect).toHaveBeenCalledTimes(1));
    const signal = inspect.mock.calls[0][2].signal;
    live.unmount();
    expect(signal.aborted).toBe(true);
  });
});

describe('manual source inspection refresh', () => {
  it('retries unchanged selectors after an unavailable result', async () => {
    inspect
      .mockRejectedValueOnce(new Error('Unavailable'))
      .mockResolvedValueOnce({ run: { status: 'SOURCE_RUN_STATUS_APPLIED' } });
    render(<SourceIntegrationReview namespace="models" />);
    openRun();
    await screen.findByRole('alert');
    fireEvent.click(screen.getByText('Inspect backend records'));
    await waitFor(() => expect(inspect).toHaveBeenCalledTimes(2));
    await screen.findByText(/Run status: SOURCE_RUN_STATUS_APPLIED/);
    expect(screen.queryByRole('alert')).toBeNull();
  });
  it('refreshes the first page after reading a continuation page', async () => {
    inspect
      .mockResolvedValueOnce({ nextPageToken: 'continuation' })
      .mockResolvedValueOnce({ registrations: [{ id: 'later' }] })
      .mockResolvedValueOnce({ registrations: [{ id: 'fresh' }] });
    render(<SourceIntegrationReview namespace="models" />);
    fireEvent.click(screen.getByText('Inspect backend records'));
    fireEvent.click(await screen.findByText('Read next page'));
    await waitFor(() => expect(inspect).toHaveBeenCalledTimes(2));
    expect(inspect.mock.calls[1][1]).toEqual({
      administrationNamespace: 'models',
      pageSize: 100,
      pageToken: 'continuation',
    });
    fireEvent.click(screen.getByText('Inspect backend records'));
    await waitFor(() => expect(inspect).toHaveBeenCalledTimes(3));
    expect(inspect.mock.calls[2][1]).toEqual({ administrationNamespace: 'models', pageSize: 100 });
  });
  it('aborts a superseded inspection and ignores its late response', async () => {
    let finish: (value: unknown) => void = () => {};
    inspect
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      )
      .mockResolvedValueOnce({ run: { status: 'SOURCE_RUN_STATUS_APPLIED' } });
    render(<SourceIntegrationReview namespace="models" />);
    openRun();
    await waitFor(() => expect(inspect).toHaveBeenCalledTimes(1));
    const oldSignal = inspect.mock.calls[0][2].signal;
    fireEvent.click(screen.getByText('Inspect backend records'));
    await waitFor(() => expect(inspect).toHaveBeenCalledTimes(2));
    expect(oldSignal.aborted).toBe(true);
    await screen.findByText(/Run status: SOURCE_RUN_STATUS_APPLIED/);
    await act(async () => {
      finish({ run: { status: 'SOURCE_RUN_STATUS_OUTCOME_UNKNOWN', recoveryRequired: true } });
    });
    expect(screen.queryByText(/Recovery required/)).toBeNull();
    expect(inspect).toHaveBeenCalledTimes(2);
  });
});
