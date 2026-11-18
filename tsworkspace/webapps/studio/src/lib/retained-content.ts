const MAX_CONTENT_BYTES = 256 * 1024;
const MAX_MEDIA_TYPE_BYTES = 127;
const MAX_FILENAME_BYTES = 256;
const MEDIA_TYPE = /^[a-z0-9][a-z0-9!#$&^_.+-]*\/[a-z0-9][a-z0-9!#$&^_.+-]*$/;
const SHA256 = /^[0-9a-f]{64}$/;
const BASE64 = /^[A-Za-z0-9+/_-]*={0,2}$/;
const TEXT_SUBTYPE_SUFFIX = /\+(json|xml|yaml)$/;
const TEXT_APPLICATION = new Set(['json', 'xml', 'yaml', 'x-yaml', 'toml', 'x-ndjson']);

export const FALLBACK_DOWNLOAD_NAME = 'retained-content.bin';

export type RetainedContentProjection =
  | {
      ok: true;
      mediaType: string;
      digest: string;
      bytes: Uint8Array;
      text?: string;
      downloadName: string;
      filenameRejected: boolean;
    }
  | { ok: false; error: string };

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function isByteArray(value: unknown): value is Uint8Array {
  return ArrayBuffer.isView(value) && Object.prototype.toString.call(value) === '[object Uint8Array]';
}

function decodeBase64(value: string): Uint8Array | undefined {
  if (!BASE64.test(value)) return undefined;
  const unpadded = value.replace(/=+$/, '');
  if (unpadded.length % 4 === 1 || (value.length > unpadded.length && value.length % 4 !== 0)) return undefined;
  const standard = unpadded.replace(/-/g, '+').replace(/_/g, '/');
  let binary: string;
  try {
    binary = atob(standard);
  } catch {
    return undefined;
  }
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  const canonical = btoa(binary).replace(/=+$/, '');
  return canonical === standard ? bytes : undefined;
}

function isRecognizedText(mediaType: string): boolean {
  const [type, subtype] = mediaType.split('/');
  if (type === 'text' || mediaType === 'image/svg+xml') return true;
  return type === 'application' && (TEXT_APPLICATION.has(subtype) || TEXT_SUBTYPE_SUFFIX.test(subtype));
}

function validLogicalFilename(value: unknown): value is string {
  if (typeof value !== 'string' || !value.trim()) return false;
  if (new TextEncoder().encode(value).length > MAX_FILENAME_BYTES) return false;
  if (value === '.' || value === '..' || /[/\\]/.test(value)) return false;
  return ![...value].some((char) => {
    const code = char.codePointAt(0) as number;
    return code <= 0x1f || (code >= 0x7f && code <= 0x9f);
  });
}

export function projectRetainedContent(
  rawContent: unknown,
  rawFilename: unknown,
): RetainedContentProjection | undefined {
  if (rawContent === undefined || rawContent === null) return undefined;
  const fail = (error: string): RetainedContentProjection => ({ ok: false, error });
  const content = record(rawContent);
  const mediaType = record(content.mediaType).value;
  if (
    typeof mediaType !== 'string' ||
    new TextEncoder().encode(mediaType).length > MAX_MEDIA_TYPE_BYTES ||
    !MEDIA_TYPE.test(mediaType)
  )
    return fail('media type is missing or is not a lowercase type/subtype without parameters');
  const digest = record(content.digest).sha256;
  if (typeof digest !== 'string' || !SHA256.test(digest))
    return fail('digest is missing or is not a lowercase SHA-256 hex value');
  const encoded = content.content;
  const bytes = typeof encoded === 'string' ? decodeBase64(encoded) : isByteArray(encoded) ? encoded : undefined;
  if (!bytes) return fail('content is missing or is neither canonical base64 nor a byte array');
  if (bytes.length === 0) return fail('content is empty');
  if (bytes.length > MAX_CONTENT_BYTES)
    return fail(`content is ${bytes.length} bytes, above the ${MAX_CONTENT_BYTES} byte limit`);
  let text: string | undefined;
  if (isRecognizedText(mediaType))
    try {
      text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes);
    } catch {
      text = undefined;
    }
  const filenameDeclared = rawFilename !== undefined && rawFilename !== null;
  const filename = record(rawFilename).value;
  const valid = validLogicalFilename(filename);
  return {
    ok: true,
    mediaType,
    digest,
    bytes,
    text,
    downloadName: valid ? filename : FALLBACK_DOWNLOAD_NAME,
    filenameRejected: filenameDeclared && !valid,
  };
}
