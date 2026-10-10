import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/api')>();
  return { ...actual, api: { ...actual.api, compileTypeLibrary: vi.fn(), typeMessages: vi.fn() } };
});

import { api } from '@/lib/api';
import type { Entity } from '@/lib/model';
import { diagnosticLocation, PayloadTypePicker, TypeLibrarySources } from './TypeLibraryPanels';

const compile = vi.mocked(api.compileTypeLibrary);
const typeMessages = vi.mocked(api.typeMessages);

function entity(kind: Entity['kind'], raw: Record<string, unknown>): Entity {
  const id = { namespace: 'acme', slug: kind === 'typeLibrary' ? 'acme.v1' : 'order.placed', version: '1' };
  return {
    kind,
    id,
    key: `${kind}:acme/${id.slug}@1`,
    title: id.slug,
    doc: '',
    annotations: [],
    fields: [],
    raw: { id, ...raw },
  } as Entity;
}

const library = entity('typeLibrary', {
  files: [
    { path: 'acme/v1/money.proto', content: 'syntax = "proto3";\npackage acme.v1;' },
    { path: 'acme/v1/order.proto', content: 'syntax = "proto3";' },
  ],
  dependencies: [{ id: { namespace: 'acme', slug: 'acme.common', version: '2' } }],
});

beforeEach(() => {
  compile.mockReset();
  typeMessages.mockReset();
});

afterEach(cleanup);

describe('diagnosticLocation', () => {
  it('formats path:line:col and drops what the compiler left out', () => {
    expect(diagnosticLocation({ path: 'a.proto', line: 3, column: 7 })).toBe('a.proto:3:7');
    expect(diagnosticLocation({ path: 'a.proto', line: 3 })).toBe('a.proto:3');
    expect(diagnosticLocation({ path: 'a.proto' })).toBe('a.proto');
  });
});

describe('TypeLibrarySources', () => {
  it('shows each file and compiles the edited text without saving it', async () => {
    compile.mockResolvedValue({
      diagnostics: [{ path: 'acme/v1/order.proto', line: 2, column: 5, message: 'unknown type Money' }],
    });
    render(<TypeLibrarySources entity={library} />);

    fireEvent.change(screen.getByLabelText('Source file'), { target: { value: '1' } });
    const editor = screen.getByLabelText('Source of acme/v1/order.proto');
    fireEvent.change(editor, { target: { value: 'syntax = "proto3";\nMoney m = 1;' } });
    expect(screen.getByText(/an agent or the CLI saves them/)).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Compile' }));
    expect(await screen.findByText('acme/v1/order.proto:2:5')).toBeTruthy();
    expect(screen.getByText(/unknown type Money/)).toBeTruthy();
    expect(compile).toHaveBeenCalledWith(
      {
        id: { namespace: 'acme', slug: 'acme.v1', version: '1' },
        files: [
          { path: 'acme/v1/money.proto', content: 'syntax = "proto3";\npackage acme.v1;' },
          { path: 'acme/v1/order.proto', content: 'syntax = "proto3";\nMoney m = 1;' },
        ],
        dependencies: [{ id: { namespace: 'acme', slug: 'acme.common', version: '2' } }],
      },
      expect.anything(),
    );
  });

  it('says so when the library compiles cleanly', async () => {
    compile.mockResolvedValue({ diagnostics: [], compatibilityViolations: [] });
    render(<TypeLibrarySources entity={library} />);
    fireEvent.click(screen.getByRole('button', { name: 'Compile' }));
    expect(await screen.findByText(/Compiles, and stays wire compatible/)).toBeTruthy();
  });

  it('lists compatibility violations and request failures', async () => {
    compile.mockResolvedValueOnce({ compatibilityViolations: [{ message: 'field 2 changed type' }] });
    render(<TypeLibrarySources entity={library} />);
    fireEvent.click(screen.getByRole('button', { name: 'Compile' }));
    expect(await screen.findByText('field 2 changed type')).toBeTruthy();

    compile.mockRejectedValueOnce(new Error('dependency acme.common@2 not found'));
    fireEvent.click(screen.getByRole('button', { name: 'Compile' }));
    expect(await screen.findByText('dependency acme.common@2 not found')).toBeTruthy();
  });
});

describe('PayloadTypePicker', () => {
  it('lists the namespace messages and shows the type url of the current payload', async () => {
    typeMessages.mockResolvedValue({ messages: [{ fullName: 'acme.v1.Money' }, { fullName: 'acme.v1.OrderPlaced' }] });
    render(
      <PayloadTypePicker
        entity={entity('event', { schema: { '@type': 'type.googleapis.com/acme.v1.OrderPlaced' } })}
      />,
    );
    const select = (await screen.findByLabelText('Payload type')) as HTMLSelectElement;
    expect(select.value).toBe('acme.v1.OrderPlaced');
    expect(typeMessages).toHaveBeenCalledWith('acme', expect.anything());

    fireEvent.change(select, { target: { value: 'acme.v1.Money' } });
    expect(screen.getByText('type.googleapis.com/acme.v1.Money')).toBeTruthy();
  });

  it('reads an undecoded tenant payload by its typeUrl', async () => {
    typeMessages.mockResolvedValue({ messages: [] });
    render(<PayloadTypePicker entity={entity('event', { schema: { typeUrl: 'type.googleapis.com/acme.v1.Gone' } })} />);
    const select = (await screen.findByLabelText('Payload type')) as HTMLSelectElement;
    expect(select.value).toBe('acme.v1.Gone');
  });

  it('stays out of the way when the namespace declares no types', async () => {
    typeMessages.mockRejectedValue(new Error('unimplemented'));
    render(<PayloadTypePicker entity={entity('event', {})} />);
    await waitFor(() => expect(typeMessages).toHaveBeenCalled());
    expect(screen.queryByLabelText('Payload type')).toBeNull();
  });
});
