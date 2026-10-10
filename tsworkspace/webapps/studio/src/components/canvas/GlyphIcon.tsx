import { Clock, Cog, MonitorSmartphone, User, Zap } from 'lucide-react';
import type { Glyph } from '@/lib/glyphs';

const GLYPH_ICON: Record<Glyph, React.ComponentType<{ className?: string }>> = {
  automation: Cog,
  persona: User,
  stream: Zap,
  ui: MonitorSmartphone,
  timeline: Clock,
};

export function GlyphIcon({ glyph, className }: { glyph: Glyph; className?: string }) {
  const Icon = GLYPH_ICON[glyph];
  return <Icon className={className} />;
}
