# Board renderer performance: why the boards are still DOM

Status: **NOT SCHEDULED.** Boards render through `@xyflow/react` 12.11
(React Flow), which is DOM plus SVG. There is no GPU-accelerated path
anywhere in the studio, and there is no measured bottleneck justifying
one. This document records what the current renderer is, what to try
before replacing it, and what a replacement would actually cost, so the
next person who finds a board slow starts from a profile instead of a
rewrite.

## Where rendering happens

`<ReactFlow>` is mounted in eight places:

- `webapps/studio/src/components/canvas/Board.tsx`
- `webapps/studio/src/components/canvas/SequenceBoard.tsx`
- `webapps/studio/src/components/canvas/DomainChartBoard.tsx`
- `webapps/studio/src/components/canvas/ContextMapBoard.tsx`
- `webapps/studio/src/components/canvas/PlanBoard.tsx`
- `webapps/studio/src/components/canvas/ScreensBoard.tsx`
- `webapps/studio/src/components/OverviewPage.tsx`
- `webapps/studio/src/components/ModelShell.tsx`

Node bodies live in `webapps/studio/src/components/canvas/StickyNode.tsx`, the
largest node component and therefore the most likely per-node render
cost. Viewport persistence is in `usePersistedViewport`.

A search for `webgpu`, `webgl`, `getContext('2d')`, and `OffscreenCanvas`
across `webapps/studio/src`, `webapps/studio/server`, and `webapps/studio/shared` returns nothing.
Every pixel on a board today is a DOM node or an SVG path.

## Measure before optimizing

The rendering strategy should not change until there is a number attached
to a real model. On the largest event model available, capture:

- Node and edge count at the point where interaction degrades.
- Frame time during pan and zoom, from the browser's performance profiler.
- Time spent in React commit versus layout and paint.
- Whether the cost is steady-state paint or a re-render storm triggered
  on viewport change.

The third measurement decides everything else. React commit cost is fixed
by memoization and costs nothing to try. Paint cost is the only thing a
GPU renderer would address. Confusing the two is how a team ends up
rewriting a renderer to solve a missing `useMemo`.

Record the result here once it exists.

## Cheaper wins to exhaust first

All of these live inside the current renderer and are reversible.

**Viewport culling.** `onlyRenderVisibleElements` is available on
`<ReactFlow>` in the version already installed, and is currently not
passed at any of the eight mount points. This is the highest-leverage
change available and it is one prop per board. It carries a real
tradeoff: culled nodes unmount, so anything that depends on measuring an
offscreen node needs checking first.

**Stable identities.** Memoize node components, and keep the `nodeTypes`
object and node `data` identity stable across renders, so React Flow is
not re-rendering every node on every viewport tick.

**Level of detail by zoom.** `StickyNode.tsx` already varies presentation
with zoom, from the badge legibility work. Extend that so a deeply
zoomed-out board skips text, icons, and badges entirely rather than
merely restyling them. At the zoom levels where node count hurts, none of
that detail is readable anyway.

**Simpler edges when zoomed out.** Edge path computation and SVG path
count scale with edge count. Straight lines instead of routed or bezier
paths below a zoom threshold removes work proportional to the part of the
graph that is hurting.

Expect these four to move the number substantially. Only if they do not is
the renderer itself the problem.

## If a GPU renderer is warranted

Candidates, ordered by how much existing behavior they preserve:

1. Keep React Flow for interaction, viewport, and selection, and move only
   the heavy layer (edges, or zoomed-out node bodies) to a canvas or WebGL
   overlay. Lowest risk, keeps the DOM contract intact, and can ship to
   one board at a time.
2. A GPU graph renderer that already carries a node and edge model, such
   as Sigma.js or a PixiJS-based layer.
3. A bespoke WebGPU renderer. Only defensible if boards are genuinely in
   the tens of thousands of nodes.

Note that a general-purpose WebGPU abstraction layer is not a candidate
here, however appealing the phrase "GPU-accelerated" is. Libraries in that
category (see references) provide a uniform GPU context across browser,
headless Node, and serverless runtimes. They carry no node or edge model,
no layout, no hit testing, and no text layout, which means adopting one is
not a renderer swap but a commitment to hand-writing everything React Flow
currently provides.

That list is concrete. A full GPU rewrite means re-solving:

- Hit testing and selection, including the additive selection behavior on
  meta, ctrl, and shift in `Board.tsx`.
- Pan and zoom, including persisted viewport restore.
- Text layout and glyph rendering. This is where the badge legibility and
  control overlap fixes live. Those problems get harder once glyphs are
  GPU-drawn, not easier.
- Accessibility of sticky content, which is currently free because the
  content is real DOM.
- Tailwind theming of nodes.
- The test suite. Vitest runs on jsdom, which has no WebGPU and no
  meaningful canvas. The board and node tests assert against rendered DOM
  and would need replacing wholesale.

## Constraints on any change here

- Headless CI and jsdom have no WebGPU. A GPU path needs a working
  fallback, or the test strategy changes alongside it.
- WebGPU browser availability is narrower than DOM and SVG. Confirm
  current support against the actual user baseline rather than assuming.
- Rendering strategy stays behind an interface the boards program against.
  No GPU concepts leak into board code, and no vertical-specific logic
  leaks into the renderer.
- Any change must be adoptable per board, so it can ship to one surface
  and be measured against the others.

## When to revisit

Reconsider a GPU renderer only when all three hold:

1. A profile on a real model shows paint or GPU-bound frame time, not
   React commit time.
2. The cheaper wins above are in place and their effect measured.
3. There is a concrete board size that users actually reach and that the
   DOM path cannot serve.

## References

- [vgpu](https://vgpu.sh/), a WebGPU abstraction for browser, headless
  Node, and serverless runtimes. Evaluated and not adopted, for the
  reasons above.
- [React Flow](https://reactflow.dev/), the current renderer.
- [WebGPU specification](https://www.w3.org/TR/webgpu/).
- [Sigma.js](https://www.sigmajs.org/), a WebGL graph renderer with a node
  and edge model.
- [PixiJS](https://pixijs.com/), a WebGL and WebGPU 2D renderer.
