// The three.js viewport: orbit, grid, auto-frame, wireframe and checker.
// It renders only when something changed, so an idle app costs nothing.

import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";

import { FLOATS_PER_VERTEX, type DecodedMesh } from "./transfer";

const SURFACE = 0xbcbcbc;

/** Viewport colours per mode, matching --viewport in style.css. */
const PALETTES = {
  light: { background: 0xefefef, gridCenter: 0xb5b5b5, grid: 0xdcdcdc },
  dark: { background: 0x171717, gridCenter: 0x484848, grid: 0x2a2a2a },
};

export class Viewer {
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera = new THREE.PerspectiveCamera(40, 1, 0.01, 1000);
  private controls: OrbitControls;
  private model = new THREE.Group();
  private grid: THREE.GridHelper | null = null;
  private palette = PALETTES.dark;
  private surface = new THREE.MeshStandardMaterial({
    color: SURFACE,
    roughness: 0.75,
    metalness: 0,
    side: THREE.DoubleSide,
    polygonOffset: true,
    polygonOffsetFactor: 1,
    polygonOffsetUnits: 1,
  });
  private wire = new THREE.MeshBasicMaterial({
    color: 0x000000,
    wireframe: true,
    transparent: true,
    opacity: 0.35,
  });
  private checker: THREE.CanvasTexture;
  private wireframe = false;
  private dirty = true;

  constructor(host: HTMLElement) {
    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    this.renderer.setClearColor(this.palette.background);
    host.appendChild(this.renderer.domElement);

    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x3a3f4b, 1.6));
    // A headlight, so whichever side faces the camera is lit.
    const headlight = new THREE.DirectionalLight(0xffffff, 1.6);
    headlight.position.set(0.4, 0.6, 1);
    this.camera.add(headlight);
    this.scene.add(this.camera, this.model);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.addEventListener("change", () => (this.dirty = true));

    this.checker = checkerTexture(this.renderer.capabilities.getMaxAnisotropy());

    new ResizeObserver(() => this.resize(host)).observe(host);
    this.resize(host);
    const loop = () => {
      this.controls.update();
      if (this.dirty) {
        this.dirty = false;
        this.renderer.render(this.scene, this.camera);
      }
      requestAnimationFrame(loop);
    };
    loop();
  }

  /** Replaces the model. `frame` re-centres the camera on it. */
  setMeshes(meshes: DecodedMesh[], frame: boolean) {
    for (const child of this.model.children) {
      if (child instanceof THREE.Mesh) {
        child.geometry.dispose();
      }
    }
    this.model.clear();
    for (const m of meshes) {
      const interleaved = new THREE.InterleavedBuffer(m.vertices, FLOATS_PER_VERTEX);
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.InterleavedBufferAttribute(interleaved, 3, 0));
      geometry.setAttribute("normal", new THREE.InterleavedBufferAttribute(interleaved, 3, 3));
      geometry.setAttribute("uv", new THREE.InterleavedBufferAttribute(interleaved, 2, 6));
      geometry.setIndex(new THREE.BufferAttribute(m.indices, 1));
      geometry.computeBoundingSphere();
      const mesh = new THREE.Mesh(geometry, this.surface);
      mesh.name = m.name;
      const wire = new THREE.Mesh(geometry, this.wire);
      wire.visible = this.wireframe;
      mesh.add(wire);
      this.model.add(mesh);
    }
    if (frame) {
      this.frame();
    }
    this.dirty = true;
  }

  frame() {
    const box = new THREE.Box3().setFromObject(this.model);
    if (box.isEmpty()) {
      return;
    }
    const sphere = box.getBoundingSphere(new THREE.Sphere());
    const radius = Math.max(sphere.radius, 1e-6);
    const distance = radius / Math.sin(THREE.MathUtils.degToRad(this.camera.fov / 2)) * 1.1;
    const direction = new THREE.Vector3(1, 0.6, 1.4).normalize();
    this.camera.position.copy(sphere.center).addScaledVector(direction, distance);
    this.camera.near = radius / 100;
    this.camera.far = radius * 100;
    this.camera.updateProjectionMatrix();
    this.controls.target.copy(sphere.center);
    this.controls.update();

    this.setGrid(niceSize(radius * 4), new THREE.Vector3(sphere.center.x, box.min.y, sphere.center.z));
  }

  setDark(dark: boolean) {
    this.palette = dark ? PALETTES.dark : PALETTES.light;
    this.renderer.setClearColor(this.palette.background);
    if (this.grid) {
      this.setGrid(this.grid.userData.size, this.grid.position.clone());
    }
    this.dirty = true;
  }

  private setGrid(size: number, position: THREE.Vector3) {
    if (this.grid) {
      this.scene.remove(this.grid);
      this.grid.dispose();
    }
    this.grid = new THREE.GridHelper(size, 20, this.palette.gridCenter, this.palette.grid);
    this.grid.userData.size = size;
    this.grid.position.copy(position);
    this.scene.add(this.grid);
    this.dirty = true;
  }

  setWireframe(on: boolean) {
    this.wireframe = on;
    this.model.traverse((o) => {
      if (o instanceof THREE.Mesh && o.material === this.wire) {
        o.visible = on;
      }
    });
    this.dirty = true;
  }

  setChecker(on: boolean) {
    this.surface.map = on ? this.checker : null;
    this.surface.color.set(on ? 0xffffff : SURFACE);
    this.surface.needsUpdate = true;
    this.dirty = true;
  }

  private resize(host: HTMLElement) {
    const { clientWidth: w, clientHeight: h } = host;
    if (w === 0 || h === 0) {
      return;
    }
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.dirty = true;
  }
}

/** 1, 2 or 5 times a power of ten, at least `x`. */
function niceSize(x: number): number {
  const p = 10 ** Math.floor(Math.log10(x));
  return [1, 2, 5, 10].map((k) => k * p).find((s) => s >= x) ?? 10 * p;
}

/** The same checker as the core's `--debug-checker`: 8×8 cells, redder
 *  towards +U and greener towards +V, with a fine grid of 64 lines. */
function checkerTexture(anisotropy: number): THREE.CanvasTexture {
  const size = 1024;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d")!;
  const image = ctx.createImageData(size, size);
  const fine = size / 64;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = Math.floor((x * 8) / size);
      const j = Math.floor((y * 8) / size);
      let k = (i + j) % 2 === 0 ? 1 : 0.5;
      if (x % fine === 0 || y % fine === 0) {
        k *= 0.8;
      }
      const o = (y * size + x) * 4;
      image.data[o] = (0.3 + (0.55 * i) / 7) * k * 255;
      image.data[o + 1] = (0.3 + (0.55 * j) / 7) * k * 255;
      image.data[o + 2] = 0.55 * k * 255;
      image.data[o + 3] = 255;
    }
  }
  ctx.putImageData(image, 0, 0);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = anisotropy;
  // glTF puts V = 0 at the top of the image, as the canvas does.
  texture.flipY = false;
  return texture;
}
