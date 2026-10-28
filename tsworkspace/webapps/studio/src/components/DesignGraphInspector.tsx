import { useState } from 'react';
import { Exact, Query } from '@/components/DecisionDocumentInspector';
import {
  getDesignGraphDetail,
  listIncomingDesignGraphSubflows,
  listSubjectDesignGraphs,
  traverseDesignGraphTopology,
} from '@/lib/api';
import { readAsOfFromUrl, readBranchFromUrl } from '@/lib/branch';
import type { CatalogSubject } from '@/lib/decision-document';
import type { Entity } from '@/lib/model';

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}
function authoredSubjects(entity: Entity): CatalogSubject[] {
  return Array.isArray(entity.raw.subjects)
    ? entity.raw.subjects.flatMap((value) => (record(value).subject ? [record(value).subject as CatalogSubject] : []))
    : [];
}

export function DesignGraphInspector({ entity, asOf: historicalAsOf }: { entity: Entity; asOf?: string }) {
  const asOf = historicalAsOf ?? readAsOfFromUrl();
  const branch = asOf ? undefined : readBranchFromUrl();
  const subjects = authoredSubjects(entity);
  const [selected, setSelected] = useState<string>();
  const subject = subjects.find((item) => JSON.stringify(item) === selected) ?? subjects[0];
  if (entity.kind !== 'designGraph') return null;
  const graph = { id: entity.id };
  const contextKey = JSON.stringify({ key: entity.key, raw: entity.raw, uid: entity.uid, branch, asOf, subject });
  return (
    <section className="flex flex-col gap-3">
      <h3 className="font-semibold">Design topology inspection</h3>
      <p>
        Declared steps, transitions and targets are authored topology, not execution evidence. Target resolution states
        come from the backend and hidden targets are indistinguishable from missing ones.
      </p>
      <section className="flex flex-col gap-2">
        <h4 className="font-semibold">Loaded authored topology</h4>
        <Exact value={{ id: entity.id, uid: entity.uid, ...entity.raw }} />
      </section>
      {subject && (
        <label className="flex flex-col gap-1">
          Exact catalog subject
          <select value={JSON.stringify(subject)} onChange={(event) => setSelected(event.target.value)}>
            {subjects.map((item, index) => (
              <option key={JSON.stringify(item)} value={JSON.stringify(item)}>
                Authored exact subject {index + 1}
              </option>
            ))}
          </select>
          <Exact value={subject} />
        </label>
      )}
      {asOf ? (
        <p>
          Retained historical declarations only. Historical topology hydration is unavailable; current resolution is not
          historical evidence.
        </p>
      ) : (
        <div key={contextKey} className="flex flex-col gap-3">
          <Query title="Design graph detail" load={(signal) => getDesignGraphDetail(graph, { branch, signal })} />
          <Query
            title="Topology traversal"
            load={(signal) => traverseDesignGraphTopology(graph, { followSubflows: true }, { branch, signal })}
          />
          <Query
            title="Incoming subflow references"
            load={(signal) => listIncomingDesignGraphSubflows(graph, { branch, signal })}
          />
          {subject && (
            <Query
              title="Design graphs for exact subject"
              load={(signal) => listSubjectDesignGraphs(subject, { branch, signal })}
            />
          )}
        </div>
      )}
    </section>
  );
}
