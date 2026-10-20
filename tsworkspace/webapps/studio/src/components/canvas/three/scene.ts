// The three.js side of the 3D views: one renderer that draws any SceneSpec.
// It owns everything WebGL needs and gives all of it back on dispose, since
// a browser keeps only a few WebGL contexts and a leaked one stays until the
// tab closes. It draws a frame only when something moved, so an idle view
// costs nothing.

import {
  BoxGeometry,
  BufferGeometry,
  CubicBezierCurve3,
  type Curve,
  CylinderGeometry,
  DirectionalLight,
  DoubleSide,
  EdgesGeometry,
  Group,
  HemisphereLight,
  Line,
  LineBasicMaterial,
  LineSegments,
  type Material,
  Mesh,
  MeshBasicMaterial,
  MeshLambertMaterial,
  type Object3D,
  OctahedronGeometry,
  PerspectiveCamera,
  PlaneGeometry,
  QuadraticBezierCurve3,
  Raycaster,
  Scene,
  SphereGeometry,
  type Texture,
  TubeGeometry,
  Vector2,
  Vector3,
  WebGLRenderer,
} from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { CSS2DObject, CSS2DRenderer } from 'three/addons/renderers/CSS2DRenderer.js';
import type { Shape, Vec3 } from '@/lib/ports';
import {
  declutter,
  flowPhases,
  framePose,
  type Region,
  regionOf,
  type SceneFile,
  type SceneFocus,
  type SceneLink,
  type SceneNode,
  type ScenePlate,
  type ScenePose,
  type SceneSpec,
  type SceneTag,
  tagState,
} from '@/lib/scene3d';
import { FLAG_BORDER, FOCUS, kindColors } from './palette';
import { paintTexture } from './paint';

export type ScenePick =
  | { type: 'node'; node: SceneNode }
  | { type: 'plate'; plate: ScenePlate }
  | { type: 'file'; file: SceneFile };

export interface SceneEvents {
  pick(pick: ScenePick | undefined, additive: boolean): void;
  hover(pick: ScenePick | undefined, at: { x: number; y: number } | undefined): void;
  /** The browser took the WebGL context away; the view cannot draw again. */
  lost(): void;
}

export type PoseName = 'front' | 'overview' | { step: number };

const FOV = 40;
const FLY_MS = 800;
// A plate wider than this keeps its text on the left part only, so the text stays sharp.
const TEXT_PLATE_W = 300;
const MAX_FLOW_LINKS = 24;
const DIM = 0.15;

const v3 = (p: Vec3) => new Vector3(p.x, p.y, p.z);
const plain = (p: Vector3): Vec3 => ({ x: p.x, y: p.y, z: p.z });

// Tags are styled inline, not with Tailwind classes: classes that only the
// 3D views use would grow the stylesheet every page loads.
const TAG_BOX =
  'border-radius:4px;border:1px solid #d4d4d8;background:rgb(255 255 255/0.9);padding:2px 6px;box-shadow:0 1px 2px rgb(0 0 0/0.05)';
const TAG_STYLE: Record<SceneTag['tone'], string> = {
  zone: `${TAG_BOX};font-size:11px;font-weight:600;color:#3f3f46`,
  ghost: 'font-size:11px;font-weight:500;color:#71717a',
  file: 'border-radius:4px;background:rgb(255 255 255/0.85);padding:0 4px;font-family:ui-monospace,monospace;font-size:10px;color:#52525b',
  name: `${TAG_BOX};border-color:#e4e4e7;font-size:10.5px;font-weight:600;color:#27272a`,
};

interface NodeView {
  node: SceneNode;
  mesh: Mesh;
  materials: MeshLambertMaterial[];
  face?: MeshBasicMaterial;
  base: number;
}

interface LinkView {
  link: SceneLink;
  material: MeshBasicMaterial;
  curve: Curve<Vector3>;
  length: number;
  base: number;
}

interface TagView {
  tag: SceneTag;
  object: CSS2DObject;
  element: HTMLDivElement;
  w: number;
  h: number;
}

export class GraphScene {
  private readonly renderer: WebGLRenderer;
  private readonly labels = new CSS2DRenderer();
  private readonly camera = new PerspectiveCamera(FOV, 1, 1, 50000);
  private readonly controls: OrbitControls;
  private readonly scene = new Scene();
  private readonly content = new Group();
  private readonly marks = new Group();
  private readonly highlight = new Group();
  private step: number | undefined;
  private readonly ray = new Raycaster();
  private readonly observer: ResizeObserver;
  private readonly reduced: boolean;
  private readonly detach: (() => void)[] = [];
  private spec?: SceneSpec;
  private focus?: SceneFocus;
  private pickables: Mesh[] = [];
  private nodes = new Map<string, NodeView>();
  private files = new Map<string, { file: SceneFile; material: MeshLambertMaterial }>();
  private links = new Map<string, LinkView>();
  private tags: TagView[] = [];
  private flow: { mesh: Mesh; curve: Curve<Vector3>; phase: number }[] = [];
  private textures = new Map<string, Texture>();
  private geometries = new Map<string, BufferGeometry>();
  private tween?: { from: ScenePose; to: ScenePose; start: number; ms: number };
  private pendingPose?: PoseName;
  private pendingFocus?: string[];
  private frame = 0;
  private hoverAt?: { x: number; y: number };
  private hoverDirty = false;
  private down?: { x: number; y: number };
  private width = 0;
  private height = 0;
  private disposed = false;

  constructor(
    private readonly host: HTMLElement,
    private readonly events: SceneEvents,
  ) {
    // Throws when the browser has no WebGL; the caller shows a plain message instead.
    this.renderer = new WebGLRenderer({ antialias: true, alpha: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    this.renderer.setClearColor(0x000000, 0);
    try {
      const canvas = this.renderer.domElement;
      // OrbitControls turns off touch scrolling on the canvas itself.
      canvas.className = 'absolute inset-0 block outline-none';
      canvas.setAttribute('aria-label', '3D view of the model');
      host.appendChild(canvas);
      const labelLayer = this.labels.domElement;
      labelLayer.className = 'pointer-events-none absolute inset-0';
      host.appendChild(labelLayer);

      this.reduced = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
      this.scene.add(new HemisphereLight(0xffffff, 0x9ca3af, 2.4));
      const sun = new DirectionalLight(0xffffff, 1.4);
      sun.position.set(0.5, 1, 0.9);
      this.scene.add(sun, this.content, this.marks, this.highlight);

      this.controls = new OrbitControls(this.camera, canvas);
      const { controls } = this;
      this.detach.push(() => controls.dispose());
      Object.assign(this.controls, {
        enableDamping: !this.reduced,
        dampingFactor: 0.12,
        screenSpacePanning: true,
        zoomToCursor: true,
        rotateSpeed: 0.7,
        minDistance: 12,
      });
      const onChange = () => this.requestRender();
      this.controls.addEventListener('change', onChange);
      this.detach.push(() => this.controls.removeEventListener('change', onChange));
      const onStart = () => {
        this.tween = undefined;
      };
      this.controls.addEventListener('start', onStart);
      this.detach.push(() => this.controls.removeEventListener('start', onStart));

      this.listen(canvas, 'pointerdown', (e) => {
        if (e.button !== 0) return;
        this.down = { x: e.clientX, y: e.clientY };
      });
      this.listen(canvas, 'pointerup', (e) => {
        if (e.button !== 0) return;
        const d = this.down;
        this.down = undefined;
        if (d && Math.hypot(e.clientX - d.x, e.clientY - d.y) <= 5) {
          this.events.pick(this.hit(e.clientX, e.clientY), e.metaKey || e.ctrlKey || e.shiftKey);
        }
      });
      this.listen(canvas, 'pointercancel', () => {
        this.down = undefined;
      });
      this.listen(canvas, 'pointermove', (e) => {
        this.hoverAt = { x: e.clientX, y: e.clientY };
        this.hoverDirty = true;
        this.requestRender();
      });
      this.listen(canvas, 'pointerleave', () => {
        this.hoverAt = undefined;
        canvas.style.cursor = '';
        this.events.hover(undefined, undefined);
      });
      const onLost = (e: Event) => {
        e.preventDefault();
        this.events.lost();
      };
      canvas.addEventListener('webglcontextlost', onLost);
      // Removed before dispose forces the context loss, so a clean unmount does not report a failure.
      this.detach.push(() => canvas.removeEventListener('webglcontextlost', onLost));

      this.observer = new ResizeObserver(() => this.resize());
      this.observer.observe(host);
      const { observer } = this;
      this.detach.push(() => observer.disconnect());
      this.resize();
    } catch (e) {
      // The caller never receives the instance, so nothing else would release the context and elements.
      for (const off of this.detach) off();
      this.renderer.dispose();
      this.renderer.forceContextLoss();
      this.renderer.domElement.remove();
      this.labels.domElement.remove();
      throw e;
    }
  }

  private listen<K extends keyof HTMLElementEventMap>(
    el: HTMLElement,
    type: K,
    fn: (e: HTMLElementEventMap[K]) => void,
  ): void {
    el.addEventListener(type, fn);
    this.detach.push(() => el.removeEventListener(type, fn));
  }

  /** Replaces the drawn content. The camera stays where it is unless `pose` asks for a preset. */
  setSpec(spec: SceneSpec, pose?: PoseName): void {
    this.clearContent();
    this.spec = spec;
    this.events.hover(undefined, undefined);
    this.hoverDirty = this.hoverAt !== undefined;
    const size = regionOf([spec.overview.region.min, spec.overview.region.max]);
    const span = Math.max(100, new Vector3().subVectors(v3(size.max), v3(size.min)).length());
    this.camera.near = Math.max(0.5, span / 4000);
    this.camera.far = span * 20;
    this.camera.updateProjectionMatrix();
    this.controls.maxDistance = span * 4;
    for (const d of spec.decor) this.addDecor(d);
    for (const p of spec.plates) this.addPlate(p);
    for (const n of spec.nodes) this.addNode(n);
    // A focus request queued before the view had a size survives a spec change for the ids that still exist.
    if (this.pendingFocus) {
      const kept = this.pendingFocus.filter((id) => this.nodes.has(id));
      this.pendingFocus = kept.length > 0 ? kept : undefined;
    }
    for (const f of spec.files) this.addFile(f);
    for (const l of spec.links) this.addLink(l);
    for (const t of spec.tags) this.addTag(t);
    if (this.focus) this.setFocus(this.focus);
    this.setStep(this.step);
    if (pose) this.goTo(pose, 0);
    this.requestRender();
  }

  setFocus(focus: SceneFocus): void {
    this.focus = focus;
    for (const [id, v] of this.nodes) {
      const opacity = focus.dimNodes && !focus.lit.has(id) ? DIM : v.base;
      for (const m of v.materials) m.opacity = opacity;
      if (v.face) v.face.opacity = opacity;
    }
    for (const [id, f] of this.files) {
      const lit = focus.lit.has(id);
      f.material.color.set(lit ? FOCUS : '#a1a1aa');
      f.material.opacity = focus.dimNodes && !lit ? 0.35 : 1;
    }
    for (const [id, l] of this.links) {
      const lit = focus.litLinks.has(id);
      l.material.opacity = lit ? 1 : focus.dimLinks ? 0.06 : l.base;
    }
    this.clearGroup(this.marks);
    this.flow = [];
    for (const id of focus.selected) {
      const v = this.nodes.get(id);
      if (v) this.marks.add(this.outline(v.node.body.shape, v.mesh.position));
    }
    for (const p of this.spec?.plates ?? []) {
      if (p.entity && focus.keys.has(p.entity.key)) {
        this.marks.add(this.outline({ type: 'box', width: p.width, height: p.height, depth: 0.2 }, v3(p.center)));
      }
    }
    // Flow dots run along the lit links, as many as fit the link's length.
    const dot = this.geometry('dot', () => new SphereGeometry(1.5, 10, 8));
    const lit = [...this.links.values()].filter((l) => focus.litLinks.has(l.link.id)).slice(0, MAX_FLOW_LINKS);
    for (const l of lit) {
      for (const phase of flowPhases(l.length)) {
        const mesh = new Mesh(dot, new MeshBasicMaterial({ color: l.link.color }));
        mesh.position.copy(l.curve.getPoint(phase));
        this.marks.add(mesh);
        this.flow.push({ mesh, curve: l.curve, phase });
      }
    }
    this.requestRender();
  }

  /** Lights one step's band, or none. */
  setStep(step: number | undefined): void {
    this.step = step;
    this.clearGroup(this.highlight);
    const s = step === undefined ? undefined : this.spec?.steps[step];
    if (s) {
      const { center, width, height, facing } = s.highlight;
      const mesh = new Mesh(
        new PlaneGeometry(width, height),
        new MeshBasicMaterial({ color: FOCUS, transparent: true, opacity: 0.08, depthWrite: false, side: DoubleSide }),
      );
      mesh.position.copy(v3(center));
      if (facing === 'y') mesh.rotation.x = -Math.PI / 2;
      this.highlight.add(mesh);
    }
    this.requestRender();
  }

  pose(name: PoseName): ScenePose | undefined {
    const spec = this.spec;
    if (!spec || this.width === 0) return undefined;
    const aspect = this.width / this.height;
    if (name === 'front') return framePose(spec.front, spec.frontDirection, aspect, FOV);
    if (name === 'overview') return framePose(spec.overview.region, spec.overview.direction, aspect, FOV);
    const step = spec.steps[name.step];
    return step ? framePose(step.region, spec.stepDirection, aspect, FOV) : undefined;
  }

  /** Flies to a preset. Before the view has a size, the preset waits for one. */
  goTo(name: PoseName, ms = FLY_MS): void {
    const p = this.pose(name);
    if (!p) {
      this.pendingPose = name;
      return;
    }
    this.pendingPose = undefined;
    this.fly(p, ms);
  }

  /** Frames these nodes and the nodes linked to them, keeping the current angle when it shows the printed faces. */
  focusOn(ids: string[]): void {
    const spec = this.spec;
    if (!spec || this.width === 0) {
      this.pendingFocus = ids;
      return;
    }
    this.pendingFocus = undefined;
    const views = ids.map((id) => this.nodes.get(id)).filter((v): v is NodeView => Boolean(v));
    if (views.length === 0) return;
    const near = new Set(ids);
    for (const l of spec.links) {
      if (l.kind !== 'edge') continue;
      if (ids.includes(l.from)) near.add(l.to);
      if (ids.includes(l.to)) near.add(l.from);
    }
    const points: Vec3[] = [];
    for (const id of near) {
      const v = this.nodes.get(id);
      if (v) points.push(v.node.body.center);
    }
    let region: Region = regionOf(points, 12);
    // A link to a far moment would leave the card a dot on screen; frame the card alone then.
    const own = regionOf(
      views.map((v) => v.node.body.center),
      40,
    );
    if (
      new Vector3().subVectors(v3(region.max), v3(region.min)).length() >
      6 * new Vector3().subVectors(v3(own.max), v3(own.min)).length()
    ) {
      region = own;
    }
    const dir = new Vector3().subVectors(this.camera.position, this.controls.target).normalize();
    const front = v3(spec.frontDirection);
    if (dir.dot(front) < 0.35) dir.copy(v3(spec.stepDirection)).normalize();
    this.fly(framePose(region, plain(dir), this.width / this.height, FOV), FLY_MS);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.frame) cancelAnimationFrame(this.frame);
    this.observer.disconnect();
    for (const off of this.detach) off();
    this.controls.dispose();
    this.clearContent();
    this.clearGroup(this.highlight);
    for (const g of this.geometries.values()) g.dispose();
    this.geometries.clear();
    this.renderer.dispose();
    // Hands the context back now instead of when the garbage collector runs.
    this.renderer.forceContextLoss();
    this.renderer.domElement.remove();
    this.labels.domElement.remove();
  }

  private fly(to: ScenePose, ms: number): void {
    if (this.reduced || ms <= 0) {
      this.camera.position.copy(v3(to.position));
      this.controls.target.copy(v3(to.target));
      this.tween = undefined;
      this.controls.update();
    } else {
      this.tween = {
        from: { position: plain(this.camera.position), target: plain(this.controls.target) },
        to,
        start: performance.now(),
        ms,
      };
    }
    this.requestRender();
  }

  private resize(): void {
    const w = this.host.clientWidth;
    const h = this.host.clientHeight;
    if (!w || !h) return;
    this.width = w;
    this.height = h;
    this.renderer.setSize(w, h, false);
    this.renderer.domElement.style.width = '100%';
    this.renderer.domElement.style.height = '100%';
    this.labels.setSize(w, h);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    if (this.pendingPose) this.goTo(this.pendingPose, 0);
    if (this.pendingFocus) this.focusOn(this.pendingFocus);
    this.requestRender();
  }

  private requestRender(): void {
    if (this.disposed || this.frame) return;
    this.frame = requestAnimationFrame(this.tick);
  }

  private readonly tick = (now: number): void => {
    this.frame = 0;
    if (this.disposed) return;
    let again = false;
    if (this.tween) {
      const { from, to, start, ms } = this.tween;
      const t = Math.min(1, Math.max(0, (now - start) / ms));
      const k = t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2;
      this.camera.position.lerpVectors(v3(from.position), v3(to.position), k);
      this.controls.target.lerpVectors(v3(from.target), v3(to.target), k);
      if (t >= 1) this.tween = undefined;
      again = true;
    }
    if (this.controls.update()) {
      again = true;
      // The camera moved under a still pointer, so the card under it may have changed.
      if (this.hoverAt) this.hoverDirty = true;
    }
    if (this.flow.length > 0 && !this.reduced) {
      for (const f of this.flow) f.mesh.position.copy(f.curve.getPoint((now / 1800 + f.phase) % 1));
      again = true;
    }
    if (this.hoverDirty) this.updateHover();
    this.renderer.render(this.scene, this.camera);
    this.layoutTags();
    this.labels.render(this.scene, this.camera);
    if (again) this.requestRender();
  };

  private hit(clientX: number, clientY: number): ScenePick | undefined {
    const r = this.renderer.domElement.getBoundingClientRect();
    if (!r.width || !r.height) return undefined;
    const ndc = new Vector2(((clientX - r.left) / r.width) * 2 - 1, -((clientY - r.top) / r.height) * 2 + 1);
    this.scene.updateMatrixWorld();
    this.camera.updateMatrixWorld();
    this.ray.setFromCamera(ndc, this.camera);
    const hit = this.ray.intersectObjects(this.pickables, false)[0];
    return hit ? (hit.object.userData.pick as ScenePick) : undefined;
  }

  private updateHover(): void {
    this.hoverDirty = false;
    const pick = this.hoverAt && !this.down ? this.hit(this.hoverAt.x, this.hoverAt.y) : undefined;
    this.renderer.domElement.style.cursor = pick && pick.type !== 'file' ? 'pointer' : '';
    this.events.hover(pick, this.hoverAt);
  }

  /** Hides the names that would overlap a more important one, so labels never pile up. */
  private layoutTags(): void {
    const focus = this.focus;
    const boxes = [];
    const byId = new Map<string, TagView>();
    const projected = new Vector3();
    for (const t of this.tags) {
      const state = focus
        ? tagState(t.tag, focus)
        : { visible: !t.tag.onlyWhenLit, priority: t.tag.priority, selected: false };
      t.element.style.outline = state.selected ? `2px solid ${FOCUS}` : '';
      if (!state.visible) {
        t.object.visible = false;
        continue;
      }
      projected.copy(t.object.position).project(this.camera);
      if (projected.z < -1 || projected.z > 1 || Math.abs(projected.x) > 1.05 || Math.abs(projected.y) > 1.05) {
        t.object.visible = false;
        continue;
      }
      const x = ((projected.x + 1) / 2) * this.width;
      const y = ((1 - projected.y) / 2) * this.height;
      if (!t.w && t.element.offsetWidth) {
        t.w = t.element.offsetWidth;
        t.h = t.element.offsetHeight;
      }
      const w = t.w || t.tag.text.length * 6.5 + 14;
      const h = t.h || (t.tag.sub ? 30 : 18);
      boxes.push({ id: t.tag.id, x: x - w / 2, y: y - h / 2, w, h, priority: state.priority });
      byId.set(t.tag.id, t);
    }
    const keep = declutter(boxes);
    for (const b of boxes) {
      const view = byId.get(b.id);
      if (view) view.object.visible = keep.has(b.id);
    }
  }

  private geometry<T extends BufferGeometry>(key: string, make: () => T): T {
    const g = this.geometries.get(key);
    if (g) return g as T;
    const made = make();
    this.geometries.set(key, made);
    return made;
  }

  private shapeGeometry(s: Shape): BufferGeometry {
    switch (s.type) {
      case 'box':
        return this.geometry(
          `box:${s.width}:${s.height}:${s.depth}`,
          () => new BoxGeometry(s.width, s.height, s.depth),
        );
      case 'sphere':
        return this.geometry(`sphere:${s.radius}`, () => new SphereGeometry(s.radius, 24, 16));
      case 'octahedron':
        return this.geometry(`octahedron:${s.radius}`, () => new OctahedronGeometry(s.radius));
      case 'cylinder':
        return this.geometry(
          `cylinder:${s.radius}:${s.height}`,
          () => new CylinderGeometry(s.radius, s.radius, s.height, 36),
        );
    }
  }

  private texture(key: string, make: () => Texture | undefined): Texture | undefined {
    const t = this.textures.get(key);
    if (t) return t;
    const made = make();
    if (made) this.textures.set(key, made);
    return made;
  }

  private addNode(node: SceneNode): void {
    const colors = kindColors(node.entity.kind);
    const border = node.flag ? FLAG_BORDER[node.flag] : colors.border;
    const base = node.synthetic ? 0.6 : 1;
    const side = new MeshLambertMaterial({ color: border, transparent: true, opacity: base });
    let face: MeshBasicMaterial | undefined;
    let material: Material | Material[] = side;
    if (node.printed && node.body.shape.type === 'box') {
      const { width, height } = node.body.shape;
      const notes = [colors.label, node.synthetic ? 'implicit' : undefined, node.flag, node.instance].filter(Boolean);
      const key = `card:${node.entity.key}:${notes.join('|')}:${width}:${height}`;
      const map = this.texture(key, () =>
        paintTexture(
          {
            width,
            height,
            bg: colors.bg,
            border,
            dashed: node.flag === 'pending' || node.flag === 'orphan' || node.synthetic,
            kicker: notes.join(' · '),
            title: node.entity.title,
            sub: node.entity.id.slug,
          },
          this.renderer.capabilities.getMaxAnisotropy(),
        ),
      );
      face = new MeshBasicMaterial({ map, color: map ? 0xffffff : colors.bg, transparent: true, opacity: base });
      // BoxGeometry orders its faces +x, -x, +y, -y, +z, -z; the +z face faces the Front preset.
      material = [side, side, side, side, face, side];
    }
    const mesh = new Mesh(this.shapeGeometry(node.body.shape), material);
    mesh.position.copy(v3(node.body.center));
    mesh.userData.pick = { type: 'node', node } satisfies ScenePick;
    this.content.add(mesh);
    this.pickables.push(mesh);
    this.nodes.set(node.id, { node, mesh, materials: [side], face, base });
  }

  private addFile(file: SceneFile): void {
    const material = new MeshLambertMaterial({ color: '#a1a1aa', transparent: true });
    const mesh = new Mesh(this.shapeGeometry(file.body.shape), material);
    mesh.position.copy(v3(file.body.center));
    mesh.userData.pick = { type: 'file', file } satisfies ScenePick;
    this.content.add(mesh);
    this.pickables.push(mesh);
    this.files.set(file.id, { file, material });
  }

  private addLink(link: SceneLink): void {
    const [a, b, c, d] = link.points.map(v3);
    const curve: Curve<Vector3> = d ? new CubicBezierCurve3(a, b, c, d) : new QuadraticBezierCurve3(a, b, c);
    const base = link.kind === 'cite' ? 0.35 : 0.6;
    const material = new MeshBasicMaterial({ color: link.color, transparent: true, opacity: base, depthWrite: false });
    const mesh = new Mesh(new TubeGeometry(curve, 32, link.radius, 6, false), material);
    this.content.add(mesh);
    this.links.set(link.id, { link, material, curve, length: curve.getLength(), base });
  }

  private addPlate(p: ScenePlate): void {
    const colors = p.kind ? kindColors(p.kind) : undefined;
    const bg = p.tint ?? colors?.bg ?? '#fafafa';
    const border = p.border ?? colors?.border ?? '#d4d4d8';
    const kicker = p.kicker ?? colors?.label;
    const anisotropy = this.renderer.capabilities.getMaxAnisotropy();
    const textW = Math.min(p.width, TEXT_PLATE_W);
    const map = this.texture(`plate:${p.id}:${p.title}:${p.sub ?? ''}:${kicker ?? ''}:${textW}:${p.height}`, () =>
      paintTexture(
        {
          width: textW,
          height: p.height,
          bg: textW === p.width ? bg : undefined,
          border: textW === p.width ? border : undefined,
          dashed: p.dashed,
          kicker,
          title: p.title,
          sub: p.sub,
          titleSize: Math.min(3.25, p.height * 0.32),
          lines: 2,
        },
        anisotropy,
      ),
    );
    let pickMesh: Mesh;
    if (textW === p.width) {
      pickMesh = new Mesh(
        new PlaneGeometry(p.width, p.height),
        new MeshBasicMaterial({ map, color: map ? 0xffffff : bg, transparent: true }),
      );
      pickMesh.position.copy(v3(p.center));
      this.content.add(pickMesh);
    } else {
      // A wide header: a plain colored plate, its border, and sharp text on its left end.
      pickMesh = new Mesh(
        new PlaneGeometry(p.width, p.height),
        new MeshBasicMaterial({ color: bg, transparent: true }),
      );
      pickMesh.position.copy(v3(p.center));
      const frame = new LineSegments(
        new EdgesGeometry(new PlaneGeometry(p.width, p.height)),
        new LineBasicMaterial({ color: border }),
      );
      frame.position.copy(pickMesh.position);
      const text = new Mesh(
        new PlaneGeometry(textW, p.height),
        new MeshBasicMaterial({ map, transparent: true, depthWrite: false }),
      );
      text.position.set(p.center.x - p.width / 2 + textW / 2, p.center.y, p.center.z + 0.05);
      this.content.add(pickMesh, frame, text);
    }
    if (p.entity) {
      pickMesh.userData.pick = { type: 'plate', plate: p } satisfies ScenePick;
      this.pickables.push(pickMesh);
    }
  }

  private addDecor(d: SceneSpec['decor'][number]): void {
    let obj: Object3D;
    switch (d.type) {
      case 'plane': {
        obj = new Mesh(
          new PlaneGeometry(d.width, d.height),
          new MeshBasicMaterial({
            color: d.color,
            transparent: true,
            opacity: d.opacity,
            depthWrite: false,
            side: DoubleSide,
          }),
        );
        if (d.facing === 'y') obj.rotation.x = -Math.PI / 2;
        obj.position.copy(v3(d.center));
        break;
      }
      case 'line':
        obj = new Line(
          new BufferGeometry().setFromPoints(d.points.map(v3)),
          new LineBasicMaterial({ color: d.color, transparent: true, opacity: d.opacity }),
        );
        break;
      case 'shell':
        obj = new Mesh(
          new CylinderGeometry(d.radius, d.radius, d.height, 48, 1, true),
          new MeshBasicMaterial({
            color: d.color,
            transparent: true,
            opacity: d.opacity,
            depthWrite: false,
            side: DoubleSide,
          }),
        );
        obj.position.copy(v3(d.center));
        break;
      case 'disc':
        obj = new Mesh(
          new CylinderGeometry(d.radius, d.radius, d.height, 48),
          new MeshLambertMaterial({ color: d.color }),
        );
        obj.position.copy(v3(d.center));
        break;
    }
    this.content.add(obj);
  }

  private addTag(tag: SceneTag): void {
    const element = document.createElement('div');
    element.style.cssText = `${TAG_STYLE[tag.tone]};white-space:nowrap`;
    element.textContent = tag.text;
    if (tag.sub) {
      const sub = document.createElement('div');
      sub.style.cssText = 'font-family:ui-monospace,monospace;font-size:9px;font-weight:400;color:#71717a';
      sub.textContent = tag.sub;
      element.appendChild(sub);
    }
    const object = new CSS2DObject(element);
    object.position.copy(v3(tag.position));
    this.content.add(object);
    this.tags.push({ tag, object, element, w: 0, h: 0 });
  }

  private outline(shape: Shape, at: Vector3): LineSegments {
    const box =
      shape.type === 'box'
        ? new BoxGeometry(shape.width + 1.6, shape.height + 1.6, shape.depth + 1.6)
        : shape.type === 'cylinder'
          ? new CylinderGeometry(shape.radius + 1.2, shape.radius + 1.2, shape.height + 1.6, 36)
          : new SphereGeometry(shape.radius + 1.6, 12, 8);
    const edges = new EdgesGeometry(box, 20);
    box.dispose();
    const lines = new LineSegments(edges, new LineBasicMaterial({ color: FOCUS }));
    lines.position.copy(at);
    return lines;
  }

  private clearGroup(group: Group): void {
    const shared = new Set(this.geometries.values());
    group.traverse((o) => {
      const mesh = o as Partial<Mesh>;
      if (mesh.geometry && !shared.has(mesh.geometry)) mesh.geometry.dispose();
      const mats = mesh.material ? ([] as Material[]).concat(mesh.material) : [];
      for (const m of mats) m.dispose();
    });
    group.clear();
  }

  private clearContent(): void {
    this.clearGroup(this.content);
    this.clearGroup(this.marks);
    for (const [key, g] of this.geometries) {
      if (!key.startsWith('dot')) {
        g.dispose();
        this.geometries.delete(key);
      }
    }
    for (const t of this.textures.values()) t.dispose();
    this.textures.clear();
    this.pickables = [];
    this.nodes.clear();
    this.files.clear();
    this.links.clear();
    this.flow = [];
    for (const t of this.tags) t.element.remove();
    this.tags = [];
  }
}
