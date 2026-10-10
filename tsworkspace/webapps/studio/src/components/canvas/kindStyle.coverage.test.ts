import { describe, expect, it } from 'vitest';
import type { EntityKind } from '@/lib/model';
import { kindStyle } from './StickyNode';

const ALL_KINDS: EntityKind[] = [
  'event',
  'command',
  'readModel',
  'processor',
  'ui',
  'persona',
  'swimlane',
  'commandSlice',
  'readModelSlice',
  'automationSlice',
  'uiSlice',
  'storyboard',
  'eventModel',
  'component',
  'externalSystem',
  'tracker',
  'boundedContext',
  'domain',
  'subdomain',
  'schema',
  'project',
  'screen',
  'term',
  'ambiguity',
  'serviceLevelIndicator',
  'serviceLevelObjective',
  'alertPolicy',
  'alertNotificationTarget',
  'typeLibrary',
];

describe('kindStyle EntityKind coverage', () => {
  it('gives every EntityKind a human label distinct from the raw camelCase id', () => {
    const rawFallbacks = ALL_KINDS.filter((k) => kindStyle(k).label === k);
    expect(rawFallbacks).toEqual([]);
  });
});
