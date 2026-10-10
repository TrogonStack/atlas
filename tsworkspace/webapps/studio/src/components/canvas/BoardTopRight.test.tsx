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
