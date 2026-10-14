import { FileCode } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { download } from '@/lib/download';
import { type SnapshotStamp, snapshotFileName } from '@/lib/snapshot';

/**
 * Saves the manifests a snapshot carries. The external entities go in their
 * own file, because they belong to other teams and are applied apart.
 */
export function DownloadYamlButton({ snapshot }: { snapshot: SnapshotStamp }) {
  const { manifests } = snapshot;
  if (!manifests) return null;
  return (
    <Button
      variant="outline"
      size="sm"
      title="Save this board as the YAML manifests trogon-atlas apply reads"
      onClick={() => {
        download(snapshotFileName(snapshot, '.yaml'), manifests.model, 'application/yaml');
        if (manifests.external) {
          download(snapshotFileName(snapshot, '.external.yaml'), manifests.external, 'application/yaml');
        }
      }}
    >
      <FileCode />
      Download YAML
    </Button>
  );
}
