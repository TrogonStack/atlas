import type { ReactNode } from 'react';
import { ExternalLink } from 'lucide-react';
import {
  metadataIssueSummary,
  projectAgentAnnotation,
  projectBadgesAnnotation,
  projectLifecycleAnnotation,
  projectResourceGroupsAnnotation,
  type EntityRefPresentation,
  type ProjectionIssue,
} from '@/lib/agent-metadata';
import type { Annotation, Model } from '@/lib/model';
import { Badge } from '@/components/ui/badge';

function MalformedBanner({ issues }: { issues: ProjectionIssue[] }) {
  const summary = metadataIssueSummary(issues);
  if (!summary) return null;
  return (
    <p className="mt-1 text-[10px] text-amber-800" data-testid="metadata-projection-issue">
      METADATA_PROJECTION: {summary}
    </p>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex gap-2 text-[10px]">
      <span className="shrink-0 uppercase tracking-wide text-muted-foreground">{label}</span>
      <span className="min-w-0 break-words">{children}</span>
    </div>
  );
}

function IconSlot({ icon }: { icon?: { kind: 'icon'; token: string } | { kind: 'fallback'; label: string } }) {
  if (!icon) return null;
  if (icon.kind === 'icon') {
    return (
      <span className="rounded bg-muted px-1 font-mono text-[9px]" title={icon.token}>
        {icon.token}
      </span>
    );
  }
  if (!icon.label) return null;
  return <span className="font-mono text-[9px] text-muted-foreground">[{icon.label}]</span>;
}

function InertLink({ url, label }: { url?: string; label: string }) {
  if (!url) return <span>{label}</span>;
  let href: string | undefined;
  try {
    const parsed = new URL(url);
    if (parsed.protocol === 'https:' && parsed.hostname && !parsed.username && !parsed.password) href = url;
  } catch {
    href = undefined;
  }
  if (!href) {
    return (
      <span>
        {label} <span className="text-amber-800">(link blocked)</span>
      </span>
    );
  }
  return (
    <a href={href} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 underline">
      {label}
      <ExternalLink className="h-3 w-3 shrink-0" />
    </a>
  );
}

function EntityRefRow({ target, onSelect }: { target: EntityRefPresentation; onSelect: (key: string) => void }) {
  switch (target.state) {
    case 'loaded':
      return (
        <button type="button" className="text-left underline" onClick={() => onSelect(target.entityKey)}>
          {target.label}
        </button>
      );
    case 'not_loaded':
      return <span>{target.label} (not loaded)</span>;
    case 'unavailable':
      return <span className="text-muted-foreground">Unavailable</span>;
    case 'malformed':
      return <span className="text-amber-800">Malformed reference</span>;
  }
}

export function AgentAnnotationDetails({ annotation }: { annotation: Annotation }) {
  const view = projectAgentAnnotation(annotation.data);
  return (
    <div className="rounded-md border border-border px-2 py-1.5 text-xs" data-testid="agent-annotation">
      <div className="font-semibold">Agent</div>
      {view.presenceOnly ? <p className="mt-1 text-muted-foreground">Declared agent (no model or tools)</p> : null}
      {view.model ? (
        <dl className="mt-1 space-y-0.5">
          {view.model.provider ? <Row label="Provider">{view.model.provider}</Row> : null}
          {view.model.name ? <Row label="Model">{view.model.name}</Row> : null}
          {view.model.version ? <Row label="Version">{view.model.version}</Row> : null}
        </dl>
      ) : null}
      {view.tools.length > 0 ? (
        <ol className="mt-2 list-decimal space-y-1 pl-4" data-testid="agent-tools">
          {view.tools.map((tool) => (
            // oxlint-disable-next-line react/no-array-index-key -- tool rows have no stable server id in read-only projection
            <li key={`${tool.name}:${tool.type}:${tool.url ?? ''}:${tool.description ?? ''}`}>
              <div className="flex flex-wrap items-center gap-1">
                <span className="font-semibold">{tool.name}</span>
                <Badge variant="outline">{tool.type}</Badge>
                <IconSlot icon={tool.icon} />
              </div>
              {tool.description ? <p className="mt-0.5 text-muted-foreground">{tool.description}</p> : null}
              {tool.url ? (
                <p className="mt-0.5">
                  <InertLink url={tool.url} label="Documentation" />
                </p>
              ) : null}
              <MalformedBanner issues={tool.issues} />
            </li>
          ))}
        </ol>
      ) : null}
      <MalformedBanner issues={view.issues} />
    </div>
  );
}

export function LifecycleAnnotationDetails({ annotation }: { annotation: Annotation }) {
  const view = projectLifecycleAnnotation(annotation.data);
  return (
    <div className="rounded-md border border-border px-2 py-1.5 text-xs" data-testid="lifecycle-annotation">
      <Badge variant="secondary">{view.status || '?'}</Badge>
      {view.doc ? <span className="ml-2 text-muted-foreground">{view.doc}</span> : null}
      {view.draft ? (
        <div className="mt-1 rounded border border-dashed px-2 py-1" data-testid="lifecycle-draft">
          {view.draft.title ? <div className="font-semibold">{view.draft.title}</div> : null}
          <p className="leading-snug">{view.draft.message}</p>
        </div>
      ) : null}
      {view.deprecatedOn ? (
        <p className="mt-1 text-[10px] text-muted-foreground" data-testid="lifecycle-deprecated-on">
          Deprecated on {view.deprecatedOn}
        </p>
      ) : null}
      <MalformedBanner issues={view.issues} />
    </div>
  );
}

export function BadgesAnnotationDetails({ annotation }: { annotation: Annotation }) {
  const view = projectBadgesAnnotation(annotation.data);
  return (
    <div className="space-y-1" data-testid="badges-annotation">
      {view.badges.map((badge) => (
        <div
          key={JSON.stringify([badge.content, badge.backgroundColor, badge.textColor, badge.icon, badge.url])}
          className="inline-flex max-w-full items-center gap-1 rounded-md border px-2 py-1 text-xs"
          style={{ backgroundColor: badge.backgroundColor, color: badge.textColor }}
        >
          <IconSlot icon={badge.icon} />
          <span className="break-words">{badge.content}</span>
          {badge.url ? <InertLink url={badge.url} label="" /> : null}
          <MalformedBanner issues={badge.issues} />
        </div>
      ))}
      <MalformedBanner issues={view.issues} />
    </div>
  );
}

export function ResourceGroupsAnnotationDetails({
  annotation,
  model,
  onSelect,
  unavailableItemIds,
}: {
  annotation: Annotation;
  model: Model;
  onSelect: (key: string) => void;
  unavailableItemIds?: ReadonlySet<string>;
}) {
  const view = projectResourceGroupsAnnotation(annotation.data, model, { unavailableItemIds });
  return (
    <div className="rounded-md border border-border px-2 py-1.5 text-xs" data-testid="resource-groups-annotation">
      <div className="font-semibold">Resource groups</div>
      <ol className="mt-1 list-decimal space-y-2 pl-4" data-testid="resource-groups">
        {view.groups.map((group) => (
          <li key={group.groupId}>
            <div className="flex flex-wrap items-center gap-1">
              <span className="font-mono text-[10px]">{group.groupId}</span>
              {group.title ? <span className="font-semibold">{group.title}</span> : null}
              {group.displayLimit !== undefined ? (
                <Badge variant="outline" title="Presentation hint only; inspector lists every stored item">
                  display limit {group.displayLimit}
                </Badge>
              ) : null}
              {group.sidebar !== undefined ? (
                <Badge variant="secondary">{group.sidebar ? 'sidebar' : 'not in sidebar'}</Badge>
              ) : null}
            </div>
            <ol className="mt-1 list-[lower-alpha] space-y-1 pl-4" data-testid={`group-items-${group.groupId}`}>
              {group.items.map((item) => (
                <li key={item.connectionId}>
                  <span className="font-mono text-[10px] text-muted-foreground">{item.connectionId}</span>
                  <div className="mt-0.5">
                    <EntityRefRow target={item.target} onSelect={onSelect} />
                  </div>
                  <MalformedBanner issues={item.issues} />
                </li>
              ))}
            </ol>
            <MalformedBanner issues={group.issues} />
          </li>
        ))}
      </ol>
      <MalformedBanner issues={view.issues} />
    </div>
  );
}
