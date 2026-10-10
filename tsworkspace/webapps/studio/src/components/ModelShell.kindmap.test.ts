import { describe, expect, it } from 'vitest';
import type { EntityKind } from '@/lib/model';
import { KIND_SHORT_TO_CAMEL } from './ModelShell.kindmap';

const ALL_ENTITY_KINDS: EntityKind[] = [
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

describe('KIND_SHORT_TO_CAMEL registry completeness', () => {
  it('every EntityKind is reachable via at least one key', () => {
    const covered = new Set(Object.values(KIND_SHORT_TO_CAMEL));
    const missing = ALL_ENTITY_KINDS.filter((k) => !covered.has(k));
    expect(missing, `missing kinds: ${missing.join(', ')}`).toHaveLength(0);
  });

  it('every value maps to a valid EntityKind', () => {
    const kindSet = new Set<string>(ALL_ENTITY_KINDS);
    for (const [key, value] of Object.entries(KIND_SHORT_TO_CAMEL)) {
      expect(kindSet.has(value), `${key} -> ${value} is not a valid EntityKind`).toBe(true);
    }
  });
});
