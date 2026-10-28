import { describe, expect, it } from 'vitest';
import { kindStyle } from '@/components/canvas/StickyNode';
import type { EntityKind } from '@/lib/model';
import { hexOfClass, kindColors } from './palette';

// A Record over EntityKind makes the compiler list every kind, so a new kind cannot skip this test.
const KINDS: Record<EntityKind, true> = {
  event: true,
  command: true,
  readModel: true,
  processor: true,
  ui: true,
  persona: true,
  swimlane: true,
  commandSlice: true,
  readModelSlice: true,
  automationSlice: true,
  uiSlice: true,
  storyboard: true,
  eventModel: true,
  component: true,
  externalSystem: true,
  tracker: true,
  boundedContext: true,
  domain: true,
  subdomain: true,
  schema: true,
  project: true,
  screen: true,
  term: true,
  ambiguity: true,
  serviceLevelIndicator: true,
  serviceLevelObjective: true,
  alertPolicy: true,
  alertNotificationTarget: true,
  typeLibrary: true,
  channel: true,
  dataStore: true,
  systemGroup: true,
  team: true,
  person: true,
  decision: true,
  document: true,
  dataProduct: true,
  specification: true,
};

describe('kindColors', () => {
  it.each(Object.keys(KINDS) as EntityKind[])('has a hex value for each Board class of %s', (kind) => {
    const style = kindStyle(kind);
    // The class goes into the result, so a failure names the class that lacks a hex value.
    expect({ [style.bg]: hexOfClass(style.bg) }).toEqual({ [style.bg]: expect.stringMatching(/^#[0-9a-f]{6}$/) });
    expect({ [style.border]: hexOfClass(style.border) }).toEqual({
      [style.border]: expect.stringMatching(/^#[0-9a-f]{6}$/),
    });
    expect(kindColors(kind).label).toBe(style.label);
  });

  it('gives undefined for a class the table does not know', () => {
    expect(hexOfClass('bg-chartreuse-500')).toBeUndefined();
  });
});
