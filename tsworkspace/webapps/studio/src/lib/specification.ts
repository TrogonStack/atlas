import { asId, type Entity, entityKey, type Model } from '@/lib/model';

export type ContractRecord = Record<string, unknown>;
export function contractRecord(value: unknown): ContractRecord {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as ContractRecord) : {};
}
export function contractList(value: unknown): ContractRecord[] {
  return Array.isArray(value) ? value.map(contractRecord) : [];
}
export function contractValue(value: unknown): string {
  if (value === undefined || value === null) return 'Undeclared';
  return typeof value === 'object' ? contractValue(contractRecord(value).value) : String(value);
}
export function isApiSpecification(entity: Entity): boolean {
  if (entity.kind !== 'specification') return false;
  const operationIds = new Set(
    contractList(entity.raw.operations)
      .map((op) => contractRecord(op.memberId).value)
      .filter((id) => typeof id === 'string' && id !== ''),
  );
  return contractList(entity.raw.exposures).some((exposure) =>
    operationIds.has(contractRecord(exposure.operationId).value),
  );
}
export function contractMember(owner: Entity, collection: string, memberId: unknown): ContractRecord | undefined {
  const value = contractRecord(memberId).value;
  return typeof value === 'string'
    ? contractList(owner.raw[collection]).find((member) => contractRecord(member.memberId).value === value)
    : undefined;
}
export function resolveContractType(model: Model, selection: ContractRecord): ContractRecord | undefined {
  const ref = contractRecord(selection.schema);
  const owner = model.byKey.get(entityKey('specification', asId(contractRecord(ref.specification).id)));
  if (
    !owner ||
    typeof ref.ownerUid !== 'string' ||
    ref.ownerUid === '' ||
    owner.uid !== ref.ownerUid ||
    ref.kind !== 'CONTRACT_MEMBER_KIND_SCHEMA' ||
    ref.expectedDigest !== undefined
  )
    return undefined;
  const schema = contractMember(owner, 'schemas', ref.memberId);
  if (!schema) return undefined;
  const definition = contractRecord(schema.definition);
  const artifactSelection = contractRecord(definition.artifact);
  if (contractValue(artifactSelection.artifactId) !== contractValue(selection.expectedArtifactId)) return undefined;
  const artifact = contractMember(owner, 'artifacts', artifactSelection.artifactId);
  if (!artifact) return undefined;
  const digest = contractRecord(selection.expectedDigest).sha256;
  if (
    typeof digest !== 'string' ||
    digest !== contractRecord(artifactSelection.expectedDigest).sha256 ||
    digest !== contractRecord(artifact.digest).sha256
  )
    return undefined;
  const expected = selection.expectedSelector;
  const actual = definition.selector;
  if ((expected === undefined) !== (actual === undefined)) return undefined;
  if (
    expected !== undefined &&
    (contractRecord(expected).syntax !== contractRecord(actual).syntax ||
      contractRecord(expected).value !== contractRecord(actual).value)
  )
    return undefined;
  return schema;
}
