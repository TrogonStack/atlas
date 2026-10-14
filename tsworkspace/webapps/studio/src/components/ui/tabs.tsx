import { Tabs as BaseTabs } from '@base-ui/react/tabs';
import { cva } from 'class-variance-authority';
import * as React from 'react';
import { cn } from '@/lib/utils';

type TabsVariant = 'segmented' | 'underline';

const TabsVariantContext = React.createContext<TabsVariant>('segmented');

const tabsListVariants = cva('', {
  variants: {
    variant: {
      segmented: 'inline-flex h-8 w-fit items-center justify-center rounded-lg bg-muted p-[3px] text-muted-foreground',
      underline: 'flex gap-1 border-b border-border',
    },
  },
});

const tabsTabVariants = cva(
  'inline-flex items-center gap-1.5 whitespace-nowrap text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 [&_svg]:size-3.5 [&_svg]:shrink-0',
  {
    variants: {
      variant: {
        segmented:
          'h-full flex-1 justify-center rounded-md px-3 py-1 text-muted-foreground hover:text-foreground data-[active]:bg-background data-[active]:text-foreground data-[active]:shadow-sm',
        underline:
          'border-b-2 border-transparent px-3 py-2 text-muted-foreground hover:bg-accent hover:text-foreground data-[active]:border-foreground data-[active]:text-foreground',
      },
    },
  },
);

export function Tabs({ className, ...props }: React.ComponentProps<typeof BaseTabs.Root>) {
  return <BaseTabs.Root className={cn('flex flex-col', className)} {...props} />;
}

export function TabsList({
  className,
  variant = 'segmented',
  ...props
}: React.ComponentProps<typeof BaseTabs.List> & { variant?: TabsVariant }) {
  return (
    <TabsVariantContext value={variant}>
      <BaseTabs.List className={cn(tabsListVariants({ variant }), className)} {...props} />
    </TabsVariantContext>
  );
}

export function TabsTab({ className, ...props }: React.ComponentProps<typeof BaseTabs.Tab>) {
  const variant = React.use(TabsVariantContext);
  return <BaseTabs.Tab className={cn(tabsTabVariants({ variant }), className)} {...props} />;
}

export function TabsPanel({ className, ...props }: React.ComponentProps<typeof BaseTabs.Panel>) {
  return <BaseTabs.Panel className={cn(className)} {...props} />;
}
