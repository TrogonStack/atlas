import { describe, expect, it } from 'vitest';
import { safeLinkHref } from './Inspector';

describe('safeLinkHref', () => {
  it('passes through https URLs unchanged', () => {
    expect(safeLinkHref('https://example.com/path?q=1')).toBe('https://example.com/path?q=1');
  });

  it('passes through http URLs unchanged', () => {
    expect(safeLinkHref('http://internal.example.com')).toBe('http://internal.example.com');
  });

  it('passes through mailto URLs unchanged', () => {
    expect(safeLinkHref('mailto:user@example.com')).toBe('mailto:user@example.com');
  });

  it('blocks javascript: URLs', () => {
    expect(safeLinkHref('javascript:alert(1)')).toBe('#');
  });

  it('blocks data: URLs', () => {
    expect(safeLinkHref('data:text/html,<script>alert(1)</script>')).toBe('#');
  });

  it('returns # for empty string', () => {
    expect(safeLinkHref('')).toBe('#');
  });

  it('returns # for the literal # string', () => {
    expect(safeLinkHref('#')).toBe('#');
  });

  it('passes through absolute relative paths', () => {
    expect(safeLinkHref('/some/path')).toBe('/some/path');
  });

  it('passes through relative paths starting with .', () => {
    expect(safeLinkHref('./relative')).toBe('./relative');
  });

  it('passes through fragment-only relative URLs', () => {
    expect(safeLinkHref('#section')).toBe('#section');
  });

  it('blocks bare protocol-less strings that are not clearly safe relative paths', () => {
    expect(safeLinkHref('ftp://files.example.com')).toBe('#');
  });
});
