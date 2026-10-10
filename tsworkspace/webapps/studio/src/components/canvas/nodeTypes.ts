import type { NodeProps, NodeTypes } from '@xyflow/react';
import type { ComponentType, JSX } from 'react';

/**
 * Register a node component in a ReactFlow `NodeTypes` map. ReactFlow's
 * `NodeTypes` declares each value as `ComponentType<NodeProps>` with an
 * untyped `data: unknown`, while our components carry concrete data shapes.
 * The single assertion here erases only the data generic at the registry
 * boundary; the component itself is still fully type-checked against its
 * own data schema.
 */
export function typedNode<T extends Record<string, unknown>>(
  component: ComponentType<NodeProps & { data: T }>,
): NodeTypes[string] {
  return component as unknown as NodeTypes[string];
}

export function typedNodeFn<T extends Record<string, unknown>>(
  component: (props: NodeProps & { data: T }) => JSX.Element | null,
): NodeTypes[string] {
  return component as unknown as NodeTypes[string];
}
