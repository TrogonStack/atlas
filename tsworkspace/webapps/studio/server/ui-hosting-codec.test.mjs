// @vitest-environment jsdom
import { createRequire } from 'node:module';
import path from 'node:path';
import { createElement } from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { buildModel } from '../src/lib/model.ts';
import { uiHostings } from '../src/lib/ui-hosting.ts';
import { UiHostingDetails } from '../src/components/UiHostingDetails.tsx';

const require = createRequire(import.meta.url);
const loader = require('@grpc/proto-loader');
const root = path.resolve(import.meta.dirname, '../../../../proto');
const definitions = loader.loadSync(path.join(root, 'trogonatlas/api/eventmodel/v1alpha1/service.proto'), { keepCase: false, longs: String, enums: String, defaults: false, oneofs: false, includeDirs: [root] });
const entityCodec = definitions['trogonatlas.eventmodel.v1alpha1.Entity'];
const version = '18446744073709551615';
const id = (slug) => ({ namespace: 'shop', slug, version });
afterEach(cleanup);

it('retains hosting order, exact owner identity, Unicode, bytes and absence through the production entity codec', () => {
  const input = {
    component: { id: id('portal'), title: 'Portal', uiHostings: [
      { connectionId: { value: 'hosting-a' }, ui: { id: id('review') }, label: { value: 'Review 🦜' } },
      { connectionId: { value: 'hosting-b' }, ui: { id: id('review') }, label: { value: 'Review 🦜' }, doc: { value: '<svg onload="bad()">\r\nwhy' } },
    ] }, system: { uid: 'persisted-owner-uid' },
  };
  const decoded = entityCodec.deserialize(entityCodec.serialize(input));
  expect(decoded).toEqual(input);
  expect(decoded.component.uiHostings[0]).not.toHaveProperty('doc');
  const model = buildModel([decoded]);
  expect(uiHostings(model.entities[0]).map((item) => [item.connectionId, item.ui.id.version, item.doc])).toEqual([
    ['hosting-a', version, undefined], ['hosting-b', version, '<svg onload="bad()">\r\nwhy'],
  ]);
  render(createElement(UiHostingDetails, { entity: model.entities[0], model, onSelect: () => {}, asOf: 'saved' }));
  expect(screen.getAllByText('Review 🦜')).toHaveLength(2);
  expect(screen.getAllByText('persisted-owner-uid')).toHaveLength(2);
  expect(document.querySelector('svg')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Check selected interaction' })).toBeNull();
  expect(screen.getByText(/Current interaction inspection is unavailable for historical views/)).toBeTruthy();
});
