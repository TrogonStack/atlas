// One SVG per validator severity, shared by every place that badges a finding.
//
// These used to be emoji. Emoji render at the font's own size and detail: on
// a zoomed-out board they blur into colored specks, while an SVG scales with
// the card and stays legible.

import { Info, OctagonAlert, TriangleAlert } from 'lucide-react';
import type { Severity } from '@/lib/issues';

const SEVERITY_ICON: Record<Severity, React.ComponentType<{ className?: string }>> = {
  error: OctagonAlert,
  warning: TriangleAlert,
  info: Info,
};

export function SeverityIcon({ severity, className }: { severity: Severity; className?: string }) {
  const Icon = SEVERITY_ICON[severity];
  return <Icon aria-hidden className={className} />;
}
