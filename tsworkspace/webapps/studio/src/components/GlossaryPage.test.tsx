import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { GlossaryPage } from './GlossaryPage';

afterEach(cleanup);

describe('glossary review page', () => {
  it('starts with explicit context selection rather than claiming an empty glossary', () => {
    render(<GlossaryPage />);
    expect(screen.getByRole('heading', { name: 'Glossary', level: 1 })).toBeTruthy();
    expect(screen.getByRole('combobox', { name: 'Bounded context' })).toBeTruthy();
    expect(screen.getByText('Choose a bounded context to inspect its vocabulary.')).toBeTruthy();
    expect(screen.queryByText('No glossary entries.')).toBeNull();
    expect(screen.queryByRole('button', { name: /approve|publish|save|delete/i })).toBeNull();
  });
});
