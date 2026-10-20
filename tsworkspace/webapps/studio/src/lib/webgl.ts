// Whether this browser can draw the 3D views. Checked before the 3D code
// loads, so a browser without WebGL never downloads three.js.
export function hasWebGL(): boolean {
  try {
    const canvas = document.createElement('canvas');
    const gl = canvas.getContext('webgl2') ?? canvas.getContext('webgl');
    if (!gl) return false;
    // The probe's context counts against the browser's small context limit, so give it back now.
    gl.getExtension('WEBGL_lose_context')?.loseContext();
    return true;
  } catch {
    return false;
  }
}
