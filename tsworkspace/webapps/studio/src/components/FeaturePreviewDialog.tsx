import { Dialog } from '@base-ui/react/dialog';
import { Switch } from '@base-ui/react/switch';
import { Tabs } from '@base-ui/react/tabs';
import { Eye, X } from 'lucide-react';
import { type RefObject, useRef, useState } from 'react';
import { FeaturePreviewIllustration } from '@/components/FeaturePreviewIllustration';
import { Button } from '@/components/ui/button';
import type { StudioFeatureFlag, StudioFeatureFlags } from '@/lib/feature-flags';

const PREVIEWS = [
  {
    id: 'laneMesh',
    title: 'Lane mesh',
    description:
      "Explore the Board's swimlanes and slice columns in a 3D layout, with relationships between cards kept visible.",
    requirement: 'Available when the model contains slices.',
  },
  {
    id: 'streamTowers',
    title: 'Stream towers',
    description:
      'See each event stream as a tower, with its events stacked together and relationships connecting the streams.',
    requirement: 'Available when events in the model belong to a swimlane.',
  },
] as const;

export function FeaturePreviewDialog({
  open,
  onOpenChange,
  featureFlags,
  onFeatureFlagChange,
  finalFocus,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  featureFlags: StudioFeatureFlags;
  onFeatureFlagChange: (feature: StudioFeatureFlag, enabled: boolean) => void;
  finalFocus: RefObject<HTMLElement | null>;
}) {
  const [selected, setSelected] = useState<StudioFeatureFlag>('laneMesh');
  const closeButton = useRef<HTMLButtonElement>(null);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Backdrop className="fixed inset-0 z-30 bg-foreground/20 backdrop-blur-[2px]" />
        <Dialog.Popup
          initialFocus={closeButton}
          finalFocus={finalFocus}
          className="fixed top-1/2 left-1/2 z-40 flex h-[min(720px,calc(100dvh-2rem))] max-h-[calc(100dvh-2rem)] w-[calc(100vw-2rem)] max-w-[1080px] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border border-border bg-card text-foreground shadow-2xl outline-none"
        >
          <header className="flex shrink-0 items-start justify-between gap-4 border-b border-border px-6 py-5">
            <div className="flex flex-col gap-1.5">
              <Dialog.Title className="text-lg font-semibold tracking-tight">Feature Preview</Dialog.Title>
              <Dialog.Description className="text-sm text-muted-foreground">
                Try new ways to explore your model. Choose the previews you want to use in this browser.
              </Dialog.Description>
            </div>
            <Dialog.Close
              ref={closeButton}
              aria-label="Close Feature Preview"
              render={<Button variant="ghost" size="icon" className="shrink-0" />}
            >
              <X />
            </Dialog.Close>
          </header>
          <Tabs.Root
            value={selected}
            onValueChange={(value) => {
              if (value === 'laneMesh' || value === 'streamTowers') setSelected(value);
            }}
            orientation="vertical"
            className="grid min-h-0 flex-1 grid-cols-1 overflow-y-auto md:grid-cols-[240px_minmax(0,1fr)] md:overflow-hidden"
          >
            <Tabs.List
              aria-label="Preview features"
              activateOnFocus
              className="flex flex-col gap-1 border-b border-border bg-background/60 py-3 pr-3 pl-5 md:overflow-y-auto md:border-r md:border-b-0"
            >
              {PREVIEWS.map((preview) => (
                <Tabs.Tab
                  key={preview.id}
                  value={preview.id}
                  aria-label={preview.title}
                  className="relative flex items-center gap-2.5 rounded-md px-3 py-3 text-left text-sm font-medium outline-none before:absolute before:inset-y-2 before:-left-3 before:w-1 before:rounded-full before:bg-primary before:opacity-0 hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring data-[active]:bg-accent data-[active]:text-accent-foreground data-[active]:before:opacity-100"
                >
                  <Eye className="size-4 shrink-0 text-primary" />
                  <span className="flex-1">{preview.title}</span>
                  <span className="text-[11px] text-muted-foreground">{featureFlags[preview.id] ? 'On' : 'Off'}</span>
                </Tabs.Tab>
              ))}
            </Tabs.List>
            {PREVIEWS.map((preview) => (
              <Tabs.Panel
                key={preview.id}
                value={preview.id}
                className="flex min-w-0 flex-col gap-5 px-5 py-6 outline-none md:overflow-y-auto md:px-7"
              >
                <div className="flex items-center justify-between gap-4">
                  <h2 className="text-xl font-semibold tracking-tight">{preview.title}</h2>
                  <div className="flex shrink-0 items-center gap-3">
                    <span className="text-sm font-medium" aria-hidden="true">
                      {featureFlags[preview.id] ? 'On' : 'Off'}
                    </span>
                    <Switch.Root
                      aria-label={`Enable ${preview.title}`}
                      checked={featureFlags[preview.id]}
                      onCheckedChange={(checked) => onFeatureFlagChange(preview.id, checked)}
                      className="inline-flex h-6 w-11 cursor-pointer items-center rounded-full bg-border p-0.5 outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 data-[checked]:bg-primary"
                    >
                      <Switch.Thumb className="size-5 rounded-full bg-card shadow-sm transition-transform data-[checked]:translate-x-5" />
                    </Switch.Root>
                  </div>
                </div>
                <FeaturePreviewIllustration feature={preview.id} />
                <p className="text-sm leading-relaxed">{preview.description}</p>
                <div className="flex flex-col gap-2 text-sm">
                  <h3 className="font-medium">How to try it</h3>
                  <p className="leading-relaxed text-muted-foreground">
                    Turn this preview on, close this window, then choose {preview.title} above your model.{' '}
                    {preview.requirement}
                  </p>
                </div>
              </Tabs.Panel>
            ))}
          </Tabs.Root>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
