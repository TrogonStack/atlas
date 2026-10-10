/**
 * Mirrors `trogon_atlas_core::validate_id_component`: ASCII alphanumerics plus
 * `-_.`, no leading dot, not "." or "..", 1-200 chars.
 *
 * URL-derived namespace tokens are user-controllable. This predicate keeps a
 * crafted link from smuggling a hostile token into list calls or titles.
 *
 * @param {unknown} value
 * @returns {boolean}
 */
export function isSafeNamespace(value) {
  if (typeof value !== 'string') return false;
  if (value.length === 0 || value.length > 200) return false;
  if (value === '.' || value === '..') return false;
  if (value.startsWith('.')) return false;
  for (const ch of value) {
    const safe =
      (ch >= 'a' && ch <= 'z') ||
      (ch >= 'A' && ch <= 'Z') ||
      (ch >= '0' && ch <= '9') ||
      ch === '-' ||
      ch === '_' ||
      ch === '.';
    if (!safe) return false;
  }
  return true;
}

/**
 * Mirrors the server's branch-name rule (`validate_branch_name` in
 * `trogon-atlas-server/src/conv.rs`): at most one '/' separator (an
 * owner-prefixed name like "alex/retention-rework"), each segment
 * validated by the same `isSafeNamespace` charset, and the reserved
 * name "meta" (case-insensitive) rejected outright since the store uses
 * it internally.
 *
 * @param {string} value
 * @returns {boolean}
 */
export function isSafeBranchName(value) {
  if (typeof value !== 'string' || value.length === 0) return false;
  if (value.toLowerCase() === 'meta') return false;
  const segments = value.split('/');
  if (segments.length > 2) return false;
  return segments.every(isSafeNamespace);
}
