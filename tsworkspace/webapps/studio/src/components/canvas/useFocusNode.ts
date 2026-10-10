import type { Node } from '@xyflow/react';
import { useReactFlow } from '@xyflow/react';
import { useEffect, useRef } from 'react';

export type FocusRequest = { pending: true; n: number } | { pending?: false; key: string; n: number };

/**
 * Pans the viewport to the node representing the requested entity, centered.
 * Fired by drawer/sidebar selections; canvas clicks never auto-pan.
 */
export function useFocusNode(focus: FocusRequest | undefined, nodes: Node[], preferType?: string) {
  const { setCenter, getZoom } = useReactFlow();
  // focus.n / focus.key are the intentional re-triggers; layout refreshes
  // must not yank the camera after the user has panned away.
  const focusKey = focus && !focus.pending ? focus.key : undefined;
  const focusN = focus?.n;
  const nodesRef = useRef(nodes);
  nodesRef.current = nodes;
  useEffect(() => {
    if (focusKey === undefined || focusN === undefined) return;
    const matches = nodesRef.current.filter((n) => (n.data as { entity?: { key: string } }).entity?.key === focusKey);
    if (matches.length === 0) return;
    const node = (preferType && matches.find((n) => n.type === preferType)) || matches[0];
    const style = (node.style ?? {}) as { width?: number | string; height?: number | string };
    const w = Number(node.width ?? style.width ?? 216) || 216;
    const h = Number(node.height ?? style.height ?? 96) || 96;
    setCenter(node.position.x + w / 2, node.position.y + h / 2, {
      duration: 600,
      zoom: Math.min(Math.max(getZoom(), 0.8), 1),
    });
  }, [focusKey, focusN, preferType, setCenter, getZoom]);
}
