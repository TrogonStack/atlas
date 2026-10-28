// Read-only projection of decoded agent/shared metadata annotations for Studio.
// Synthetic fixtures and server decodeAnnotations output share this wire shape.

import { asId, entityKey, type EntityKind, type Model, wireKind } from '@/lib/model';

export type ProjectionIssue = { code: string; detail: string };

function decodeIssues(data: Record<string, unknown>): ProjectionIssue[] {
  return typeof data['@decodeError'] === 'string' ? [{ code: 'METADATA_MALFORMED', detail: data['@decodeError'] }] : [];
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function list(value: unknown, label: string, issues: ProjectionIssue[]): unknown[] {
  if (value === undefined || value === null) return [];
  if (Array.isArray(value)) return value;
  issues.push({ code: 'METADATA_MALFORMED', detail: `${label} is not a collection` });
  return [];
}

/** Required string wrapper `{ value }` or plain string from expanded JSON fixtures. */
export function readRequiredText(
  field: unknown,
  label: string,
): { ok: true; value: string } | { ok: false; issue: ProjectionIssue } {
  if (field === undefined || field === null) {
    return { ok: false, issue: { code: 'METADATA_ABSENT', detail: `${label} is missing` } };
  }
  if (typeof field === 'string') {
    return field.length > 0
      ? { ok: true, value: field }
      : { ok: false, issue: { code: 'METADATA_BLANK', detail: `${label} is blank` } };
  }
  const v = record(field).value;
  if (typeof v !== 'string' || v.length === 0) {
    return { ok: false, issue: { code: 'METADATA_MALFORMED', detail: `${label} is malformed` } };
  }
  return { ok: true, value: v };
}

/** Optional text: absent is fine; present empty or malformed is not. */
export function readOptionalText(
  field: unknown,
  label: string,
): { ok: true; value?: string } | { ok: false; issue: ProjectionIssue } {
  if (field === undefined || field === null) return { ok: true };
  if (typeof field === 'string') {
    return field.length > 0
      ? { ok: true, value: field }
      : { ok: false, issue: { code: 'METADATA_BLANK', detail: `${label} is blank` } };
  }
  const v = record(field).value;
  if (typeof v !== 'string' || v.length === 0) {
    return { ok: false, issue: { code: 'METADATA_MALFORMED', detail: `${label} is malformed` } };
  }
  return { ok: true, value: v };
}

const ICON_TOKEN = /^[A-Za-z0-9][A-Za-z0-9_.:-]*$/;

export function iconPresentation(
  token: string | undefined,
): { kind: 'icon'; token: string } | { kind: 'fallback'; label: string } {
  if (!token) return { kind: 'fallback', label: '' };
  if (token.length <= 128 && ICON_TOKEN.test(token)) return { kind: 'icon', token };
  return { kind: 'fallback', label: token };
}

const HEX_COLOR = /^#[0-9A-Fa-f]{6}$/;

export function readHexColor(
  field: unknown,
  label: string,
): { ok: true; value: string } | { ok: false; issue: ProjectionIssue } {
  const text = readRequiredText(field, label);
  if (!text.ok) return text;
  if (!HEX_COLOR.test(text.value)) {
    return { ok: false, issue: { code: 'METADATA_MALFORMED', detail: `${label} is not a valid hex color` } };
  }
  return text;
}

export type EntityRefPresentation =
  | { state: 'loaded'; entityKey: string; label: string }
  | { state: 'not_loaded'; label: string }
  | { state: 'unavailable' }
  | { state: 'malformed'; issue: ProjectionIssue };

export function exactRefLabel(kind: EntityKind, id: { namespace: string; slug: string; version: string }): string {
  return `${kind}:${id.namespace}/${id.slug}@${id.version}`;
}

export function presentEntityRef(
  model: Model,
  target: unknown,
  options?: { unavailable?: boolean },
): EntityRefPresentation {
  if (options?.unavailable) return { state: 'unavailable' };
  const body = record(target);
  const kind = wireKind(body.kind);
  const rawId = record(body.id);
  const rawVersion = rawId.version;
  const version =
    typeof rawVersion === 'number' && Number.isSafeInteger(rawVersion)
      ? String(rawVersion)
      : typeof rawVersion === 'string'
        ? rawVersion
        : '';
  const exactVersion =
    version.length <= 20 && /^[1-9][0-9]*$/.test(version) && BigInt(version) <= BigInt('18446744073709551615');
  const id = asId(body.id);
  if (
    !kind ||
    typeof rawId.namespace !== 'string' ||
    !rawId.namespace ||
    typeof rawId.slug !== 'string' ||
    !rawId.slug ||
    !exactVersion
  ) {
    return {
      state: 'malformed',
      issue: { code: 'METADATA_MALFORMED', detail: 'Entity reference is malformed' },
    };
  }
  const label = exactRefLabel(kind, id);
  const key = entityKey(kind, id);
  const loaded = model.entities.some((e) => e.key === key);
  return loaded ? { state: 'loaded', entityKey: key, label } : { state: 'not_loaded', label };
}

export interface AgentModelView {
  provider?: string;
  name?: string;
  version?: string;
}

export interface AgentToolView {
  name: string;
  type: string;
  description?: string;
  icon?: ReturnType<typeof iconPresentation>;
  url?: string;
  issues: ProjectionIssue[];
}

export interface AgentAnnotationViewModel {
  presenceOnly: boolean;
  model?: AgentModelView;
  tools: AgentToolView[];
  issues: ProjectionIssue[];
}

export function projectAgentAnnotation(data: Record<string, unknown>): AgentAnnotationViewModel {
  const issues = decodeIssues(data);
  const modelField = data.model;
  let modelView: AgentModelView | undefined;
  if (modelField !== undefined && modelField !== null) {
    const m = record(modelField);
    const provider = readOptionalText(m.provider, 'Model provider');
    const name = readOptionalText(m.name, 'Model name');
    const version = readOptionalText(m.version, 'Model version');
    for (const part of [provider, name, version]) {
      if (!part.ok) issues.push(part.issue);
    }
    if (provider.ok && name.ok && version.ok) {
      const hasAny = provider.value || name.value || version.value;
      if (!hasAny) {
        issues.push({ code: 'METADATA_MALFORMED', detail: 'Agent model is present but empty' });
      } else {
        modelView = {
          provider: provider.value,
          name: name.value,
          version: version.value,
        };
      }
    }
  }

  const tools: AgentToolView[] = [];
  for (const raw of list(data.tools, 'Tools', issues)) {
    const t = record(raw);
    const toolIssues: ProjectionIssue[] = [];
    const name = readRequiredText(t.name, 'Tool name');
    const type = readRequiredText(t.type, 'Tool type');
    if (!name.ok) toolIssues.push(name.issue);
    if (!type.ok) toolIssues.push(type.issue);
    const description = readOptionalText(t.description, 'Tool description');
    if (!description.ok) toolIssues.push(description.issue);
    const iconText = readOptionalText(t.icon, 'Tool icon');
    if (!iconText.ok) toolIssues.push(iconText.issue);
    const url = readOptionalText(t.url, 'Tool URL');
    if (!url.ok) toolIssues.push(url.issue);
    if (name.ok && type.ok) {
      tools.push({
        name: name.value,
        type: type.value,
        description: description.ok ? description.value : undefined,
        icon: iconText.ok && iconText.value ? iconPresentation(iconText.value) : undefined,
        url: url.ok ? url.value : undefined,
        issues: toolIssues,
      });
    } else {
      tools.push({
        name: name.ok ? name.value : '?',
        type: type.ok ? type.value : '?',
        issues: toolIssues,
      });
    }
  }

  return {
    presenceOnly: modelView === undefined && tools.length === 0 && issues.length === 0,
    model: modelView,
    tools,
    issues,
  };
}

export interface DraftNoticeView {
  title?: string;
  message: string;
}

export interface LifecycleAnnotationViewModel {
  status: string;
  doc?: string;
  draft?: DraftNoticeView;
  deprecatedOn?: string;
  issues: ProjectionIssue[];
}

export function projectLifecycleAnnotation(data: Record<string, unknown>): LifecycleAnnotationViewModel {
  const issues = decodeIssues(data);
  const status =
    typeof data.status === 'string'
      ? data.status
      : (() => {
          issues.push({ code: 'METADATA_MALFORMED', detail: 'Lifecycle status is malformed' });
          return '';
        })();
  const doc = readOptionalText(data.doc, 'Lifecycle doc');
  if (!doc.ok) issues.push(doc.issue);

  let draft: DraftNoticeView | undefined;
  if (data.draft !== undefined && data.draft !== null) {
    const d = record(data.draft);
    const title = readOptionalText(d.title, 'Draft title');
    const message = readRequiredText(d.message, 'Draft message');
    if (!title.ok) issues.push(title.issue);
    if (!message.ok) issues.push(message.issue);
    if (message.ok) {
      draft = { title: title.ok ? title.value : undefined, message: message.value };
    }
  }

  let deprecatedOn: string | undefined;
  if (data.deprecatedOn !== undefined || data.deprecated_on !== undefined) {
    const field = data.deprecatedOn ?? data.deprecated_on;
    const date = readRequiredText(field, 'Deprecation date');
    if (!date.ok) issues.push(date.issue);
    else deprecatedOn = date.value;
  }

  return {
    status,
    doc: doc.ok ? doc.value : undefined,
    draft,
    deprecatedOn,
    issues,
  };
}

export interface BadgeView {
  content: string;
  backgroundColor: string;
  textColor: string;
  icon?: ReturnType<typeof iconPresentation>;
  url?: string;
  issues: ProjectionIssue[];
}

export interface BadgesAnnotationViewModel {
  badges: BadgeView[];
  issues: ProjectionIssue[];
}

export function projectBadgesAnnotation(data: Record<string, unknown>): BadgesAnnotationViewModel {
  const issues = decodeIssues(data);
  const badges: BadgeView[] = [];
  for (const raw of list(data.badges, 'Badges', issues)) {
    const b = record(raw);
    const badgeIssues: ProjectionIssue[] = [];
    const content = readRequiredText(b.content, 'Badge content');
    const backgroundColor = readHexColor(b.backgroundColor ?? b.background_color, 'Badge background color');
    const textColor = readHexColor(b.textColor ?? b.text_color, 'Badge text color');
    for (const part of [content, backgroundColor, textColor]) {
      if (!part.ok) badgeIssues.push(part.issue);
    }
    const iconText = readOptionalText(b.icon, 'Badge icon');
    if (!iconText.ok) badgeIssues.push(iconText.issue);
    const url = readOptionalText(b.url, 'Badge URL');
    if (!url.ok) badgeIssues.push(url.issue);
    if (content.ok && backgroundColor.ok && textColor.ok) {
      badges.push({
        content: content.value,
        backgroundColor: backgroundColor.value,
        textColor: textColor.value,
        icon: iconText.ok && iconText.value ? iconPresentation(iconText.value) : undefined,
        url: url.ok ? url.value : undefined,
        issues: badgeIssues,
      });
    } else {
      issues.push(...badgeIssues);
    }
  }
  return { badges, issues };
}

export interface ResourceGroupItemView {
  connectionId: string;
  target: EntityRefPresentation;
  issues: ProjectionIssue[];
}

export interface ResourceGroupView {
  groupId: string;
  title?: string;
  displayLimit?: number;
  sidebar?: boolean;
  items: ResourceGroupItemView[];
  issues: ProjectionIssue[];
}

export interface ResourceGroupsAnnotationViewModel {
  groups: ResourceGroupView[];
  issues: ProjectionIssue[];
}

export function readConnectionId(
  field: unknown,
  label: string,
): { ok: true; value: string } | { ok: false; issue: ProjectionIssue } {
  if (field === undefined || field === null) {
    return { ok: false, issue: { code: 'METADATA_MALFORMED', detail: `${label} is missing` } };
  }
  const v = record(field).value;
  if (typeof v !== 'string' || v.length === 0) {
    return { ok: false, issue: { code: 'METADATA_MALFORMED', detail: `${label} is malformed` } };
  }
  return { ok: true, value: v };
}

export function projectResourceGroupsAnnotation(
  data: Record<string, unknown>,
  model: Model,
  options?: { unavailableItemIds?: ReadonlySet<string> },
): ResourceGroupsAnnotationViewModel {
  const issues = decodeIssues(data);
  const groups: ResourceGroupView[] = [];
  for (const rawGroup of list(data.groups, 'Resource groups', issues)) {
    const g = record(rawGroup);
    const groupIssues: ProjectionIssue[] = [];
    const groupId = readConnectionId(g.groupId ?? g.group_id, 'Group ID');
    if (!groupId.ok) groupIssues.push(groupId.issue);
    const title = readOptionalText(g.title, 'Group title');
    if (!title.ok) groupIssues.push(title.issue);

    let displayLimit: number | undefined;
    const limitField = g.displayLimit ?? g.display_limit;
    if (limitField !== undefined && limitField !== null) {
      const lv = record(limitField).value;
      if (typeof lv !== 'number' || !Number.isInteger(lv) || lv < 1 || lv > 500) {
        groupIssues.push({ code: 'METADATA_MALFORMED', detail: 'Display limit is malformed' });
      } else {
        displayLimit = lv;
      }
    }

    let sidebar: boolean | undefined;
    if (g.sidebar !== undefined && g.sidebar !== null) {
      if (typeof g.sidebar === 'boolean') sidebar = g.sidebar;
      else groupIssues.push({ code: 'METADATA_MALFORMED', detail: 'Sidebar preference is malformed' });
    }

    const items: ResourceGroupItemView[] = [];
    for (const rawItem of list(g.items, 'Resource group items', groupIssues)) {
      const item = record(rawItem);
      const itemIssues: ProjectionIssue[] = [];
      const connectionId = readConnectionId(item.connectionId ?? item.connection_id, 'Item ID');
      if (!connectionId.ok) itemIssues.push(connectionId.issue);
      const unavailable = connectionId.ok && options?.unavailableItemIds?.has(connectionId.value) ? true : undefined;
      const target = presentEntityRef(model, item.target, { unavailable });
      if (target.state === 'malformed') itemIssues.push(target.issue);
      items.push({
        connectionId: connectionId.ok ? connectionId.value : '?',
        target,
        issues: itemIssues,
      });
      groupIssues.push(...itemIssues);
    }

    groups.push({
      groupId: groupId.ok ? groupId.value : '?',
      title: title.ok ? title.value : undefined,
      displayLimit,
      sidebar,
      items,
      issues: groupIssues,
    });
    issues.push(...groupIssues);
  }
  return { groups, issues };
}

export function metadataIssueSummary(issues: ProjectionIssue[]): string | undefined {
  if (issues.length === 0) return undefined;
  return issues.map((i) => i.detail).join('; ');
}
