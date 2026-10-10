import { describe, expect, it } from 'vitest';
import { isSafeBranchName, isSafeNamespace } from './safe-namespace.mjs';

describe('isSafeNamespace', () => {
  it('mirrors rust validate_id_component accept/reject set', () => {
    for (const v of ['orders', 'order.placed', 'order-placed', 'a_b', 'v2']) {
      expect(isSafeNamespace(v), `should accept ${v}`).toBe(true);
    }
    for (const v of ['', '.', '..', '.git', 'a/b', 'a b', 'a\nb', '../etc']) {
      expect(isSafeNamespace(v), `should reject ${v}`).toBe(false);
    }
    expect(isSafeNamespace('a'.repeat(200))).toBe(true);
    expect(isSafeNamespace('a'.repeat(201))).toBe(false);
  });

  // isSafeBranchName already guards typeof; isSafeNamespace must too so a
  // non-string body/query value becomes a clean false (400) rather than a
  // TypeError 500. Rust always takes &str at the boundary.
  it('returns false for non-string values without throwing', () => {
    for (const v of [123, null, undefined, true, {}, ['a']]) {
      expect(() => isSafeNamespace(/** @type {any} */ (v))).not.toThrow();
      expect(isSafeNamespace(/** @type {any} */ (v))).toBe(false);
    }
  });
});

describe('isSafeBranchName', () => {
  it('matches rust validate_branch_name reserved and segment rules', () => {
    expect(isSafeBranchName('feature-x')).toBe(true);
    expect(isSafeBranchName('alex/retention-rework')).toBe(true);
    expect(isSafeBranchName('meta')).toBe(false);
    expect(isSafeBranchName('META')).toBe(false);
    expect(isSafeBranchName('a/b/c')).toBe(false);
    expect(isSafeBranchName('')).toBe(false);
    expect(isSafeBranchName(/** @type {any} */ (null))).toBe(false);
  });
});
