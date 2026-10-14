import { Menu as BaseMenu } from '@base-ui/react/menu';
import type * as React from 'react';
import { cn } from '@/lib/utils';

export const DropdownMenu = BaseMenu.Root;
export const DropdownMenuTrigger = BaseMenu.Trigger;

export function DropdownMenuContent({
  className,
  align = 'end',
  side = 'bottom',
  sideOffset = 4,
  ...props
}: React.ComponentProps<typeof BaseMenu.Popup> &
  Pick<React.ComponentProps<typeof BaseMenu.Positioner>, 'align' | 'side' | 'sideOffset'>) {
  return (
    <BaseMenu.Portal>
      <BaseMenu.Positioner align={align} side={side} sideOffset={sideOffset} className="z-20 outline-none">
        <BaseMenu.Popup
          className={cn(
            'min-w-44 rounded-md border border-border bg-card p-1 text-xs shadow-md outline-none',
            className,
          )}
          {...props}
        />
      </BaseMenu.Positioner>
    </BaseMenu.Portal>
  );
}

export function DropdownMenuItem({ className, ...props }: React.ComponentProps<typeof BaseMenu.Item>) {
  return (
    <BaseMenu.Item
      className={cn(
        'flex select-none items-center gap-2 rounded px-2 py-1.5 outline-none data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground data-[disabled]:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0',
        className,
      )}
      {...props}
    />
  );
}
