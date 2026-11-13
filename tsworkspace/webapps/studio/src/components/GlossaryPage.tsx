import { BookOpen } from 'lucide-react';
import { buttonVariants } from '@/components/ui/button';
import { readBranchFromUrl, withBranchQuery } from '@/lib/branch';

export function GlossaryPage() {
  return (
    <div className="flex min-h-screen flex-col bg-background">
      <header className="flex flex-wrap items-center gap-4 border-b border-border bg-card px-6 py-4">
        <BookOpen className="size-5 text-muted-foreground" aria-hidden="true" />
        <div className="flex flex-1 flex-col gap-1">
          <h1 className="text-lg font-semibold">Glossary</h1>
          <p className="text-xs text-muted-foreground">Vocabulary, evidence and review within each bounded context.</p>
        </div>
        <a
          href={withBranchQuery('/', readBranchFromUrl())}
          className={buttonVariants({ variant: 'outline', size: 'sm' })}
        >
          Overview
        </a>
      </header>
      <main className="flex flex-1 flex-col gap-6 p-6">
        <label className="flex max-w-sm flex-col gap-2 text-sm font-medium">
          Bounded context
          <select className="h-9 rounded-md border border-input bg-card px-3 text-sm">
            <option value="">Choose a bounded context</option>
          </select>
        </label>
        <p className="text-sm text-muted-foreground">Choose a bounded context to inspect its vocabulary.</p>
      </main>
    </div>
  );
}
