// The corner used to be first-come-first-served: ModelShell anchored the
// validator summary there and each board anchored its own control there too,
// so they rendered on top of each other. These tests hold the row together.

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { BoardTopRight, BoardTopRightHost } from './BoardTopRight';

afterEach(cleanup);

describe('BoardTopRight', () => {
  it('puts the shell corner and a board control in one container', () => {
    render(
      <BoardTopRightHost corner={<span>summary</span>}>
        <BoardTopRight>
          <span>board control</span>
        </BoardTopRight>
      </BoardTopRightHost>,
    );
    const row = screen.getByTestId('board-top-right');
    expect(row.contains(screen.getByText('summary'))).toBe(true);
    expect(row.contains(screen.getByText('board control'))).toBe(true);
  });

  it('keeps the summary last so a board control cannot cover it', () => {
    render(
      <BoardTopRightHost corner={<span>summary</span>}>
        <BoardTopRight>
          <span>board control</span>
        </BoardTopRight>
      </BoardTopRightHost>,
    );
    // Flex `order`, not DOM order, decides what sits in the corner.
    expect(screen.getByText('summary').parentElement?.className).toContain('order-last');
  });

  it('seats the view switcher in the same row as the corner, so neither is pushed below the other', () => {
    render(
      <BoardTopRightHost corner={<span>summary</span>} switcher={<span>switcher</span>}>
        <BoardTopRight>
          <span>board control</span>
        </BoardTopRight>
      </BoardTopRightHost>,
    );
    const toolbar = screen.getByTestId('board-toolbar');
    expect(toolbar.contains(screen.getByText('switcher'))).toBe(true);
    expect(toolbar.contains(screen.getByTestId('board-top-right'))).toBe(true);
    // The breakpoint offset is gone: the row hugs the top edge at every width.
    expect(toolbar.className).toContain('top-2');
    expect(toolbar.className).not.toContain('top-12');
  });

  it('renders a board control in place when no shell provides the row', () => {
    render(
      <BoardTopRight>
        <span>board control</span>
      </BoardTopRight>,
    );
    expect(screen.queryByTestId('board-top-right')).toBeNull();
    expect(screen.getByText('board control')).toBeDefined();
  });
});
