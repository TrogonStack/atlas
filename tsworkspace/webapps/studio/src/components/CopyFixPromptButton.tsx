import { CopyContextButton } from '@/components/CopyContextButton';
import { readBranchFromUrl } from '@/lib/branch';
import type { Issue } from '@/lib/issues';
import type { Model } from '@/lib/model';
import { validationFixPrompt } from '@/lib/validation-context';

export function CopyFixPromptButton({ model, issue, momentKey }: { model: Model; issue: Issue; momentKey?: string }) {
  return (
    <CopyContextButton
      label="Copy fix prompt"
      copiedLabel="Fix prompt copied"
      title="Copy a ready-to-paste request with this finding, model context, and instructions to fix it."
      className="text-foreground"
      getText={() =>
        validationFixPrompt(model, issue, {
          branch: readBranchFromUrl(),
          momentKey,
          scopePath: window.location.pathname,
        })
      }
    />
  );
}
