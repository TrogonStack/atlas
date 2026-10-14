import type { LucideIcon } from 'lucide-react';
import type * as React from 'react';
import { cn } from '@/lib/utils';

type EmptyStateProps = {
  icon: LucideIcon;
  title: string;
  children?: React.ReactNode;
  hint?: React.ReactNode;
  className?: string;
};

export function EmptyState({ icon: Icon, title, children, hint, className }: EmptyStateProps) {
  return (
    <div className={cn('flex max-w-sm flex-col items-center gap-3 text-center', className)}>
      <div className="flex size-11 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Icon className="size-5" aria-hidden="true" />
      </div>
      <div className="flex flex-col gap-1">
        <p className="text-sm font-semibold text-foreground">{title}</p>
        {children ? <p className="text-xs leading-relaxed text-muted-foreground">{children}</p> : null}
      </div>
      {hint ? (
        <p className="rounded-md border border-dashed border-border px-3 py-1.5 text-[11px] leading-relaxed text-muted-foreground">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

export function BoardEmptyState(props: EmptyStateProps) {
  return (
    <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-6">
      <EmptyState
        {...props}
        className={cn(
          'pointer-events-auto rounded-xl border border-border bg-card/95 px-8 py-6 shadow-sm',
          props.className,
        )}
      />
    </div>
  );
}
