// Decodes the binary mesh buffers from the core. The layout is documented in
// app/src-tauri/src/transfer.rs; keep the two in sync.

export const FLOATS_PER_VERTEX = 8; // position xyz, normal xyz, uv
const FLAG_HAS_UV = 1;

export interface DecodedMesh {
  name: string;
  /** Interleaved, FLOATS_PER_VERTEX per vertex. */
  vertices: Float32Array;
  indices: Uint32Array;
  hasUv: boolean;
}

export function decode(buffer: ArrayBuffer): DecodedMesh[] {
  const view = new DataView(buffer);
  let at = 0;
  const u32 = () => {
    const v = view.getUint32(at, true);
    at += 4;
    return v;
  };
  const names = new TextDecoder();
  const meshes: DecodedMesh[] = [];
  for (let count = u32(); count > 0; count--) {
    const vertexCount = u32();
    const indexCount = u32();
    const flags = u32();
    const nameLength = u32();
    const name = names.decode(new Uint8Array(buffer, at, nameLength));
    at += Math.ceil(nameLength / 4) * 4;
    const vertices = new Float32Array(buffer, at, vertexCount * FLOATS_PER_VERTEX);
    at += vertices.byteLength;
    const indices = new Uint32Array(buffer, at, indexCount);
    at += indices.byteLength;
    meshes.push({ name, vertices, indices, hasUv: (flags & FLAG_HAS_UV) !== 0 });
  }
  return meshes;
}
