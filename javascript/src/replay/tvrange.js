// Video frames in TV-range BT.709 YUV, the levels players expect.
//
// A VideoFrame made from a canvas is RGB, and the browser picks how to turn
// it into YUV for the encoder: Chrome makes full-range YUV (tagged sRGB), which
// players that ignore the full-range flag (Firefox, many desktop players)
// show too dark, with the shadows crushed. So the exporter converts each
// picture itself, on the GPU (WebGL2): 4:2:0 YUV at 16-235 (Y) and 16-240
// (U, V) with the BT.709 matrix, tagged as such, which every player shows
// alike.

const VERTEX = `#version 300 es
in vec2 pos;
void main() { gl_Position = vec4(pos, 0.0, 1.0); }`;

// The output is a W/4-wide RGBA picture, each pixel 4 bytes of an I420
// frame: H rows of Y, then H/4 rows of U and H/4 of V, each row holding two
// of the plane's W/2-byte rows. Memory row r of readPixels is
// gl_FragCoord.y = r + 0.5; source row 0 is the canvas' top.
const FRAGMENT = `#version 300 es
precision highp float;
uniform sampler2D src;
uniform int W;
uniform int H;
out vec4 o;
const vec3 K = vec3(0.2126, 0.7152, 0.0722);
vec3 at(int x, int y) { return texelFetch(src, ivec2(x, y), 0).rgb; }
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  vec4 r;
  if (p.y < H) {
    for (int i = 0; i < 4; i++) r[i] = (16.0 + 219.0 * dot(at(p.x * 4 + i, p.y), K)) / 255.0;
  } else {
    int r0 = p.y - H;
    bool isV = r0 >= H / 4;
    int q = W / 8;
    int y2 = ((isV ? r0 - H / 4 : r0) * 2 + (p.x >= q ? 1 : 0)) * 2;
    int x0 = (p.x % q) * 4;
    for (int i = 0; i < 4; i++) {
      int x2 = (x0 + i) * 2;
      vec3 c = (at(x2, y2) + at(x2 + 1, y2) + at(x2, y2 + 1) + at(x2 + 1, y2 + 1)) * 0.25;
      float y = dot(c, K);
      float d = isV ? (c.r - y) / 1.5748 : (c.b - y) / 1.8556;
      r[i] = (128.0 + 224.0 * d) / 255.0;
    }
  }
  o = r;
}`;

export const TV_BT709 = { primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', fullRange: false };

/**
 * Converts W x H pictures (W a multiple of 8, H of 4) to TV-range BT.709
 * I420 VideoFrames. Null if this browser has no WebGL2.
 */
export function createTvRangeConverter(W, H) {
  if (W % 8 || H % 4 || typeof OffscreenCanvas === 'undefined') return null;
  const canvas = new OffscreenCanvas(W / 4, H + H / 2);
  const gl = canvas.getContext('webgl2', { antialias: false, depth: false, premultipliedAlpha: false });
  if (!gl) return null;

  const shader = (type, source) => {
    const s = gl.createShader(type);
    gl.shaderSource(s, source);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  };
  const program = gl.createProgram();
  gl.attachShader(program, shader(gl.VERTEX_SHADER, VERTEX));
  gl.attachShader(program, shader(gl.FRAGMENT_SHADER, FRAGMENT));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
  gl.useProgram(program);

  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  const pos = gl.getAttribLocation(program, 'pos');
  gl.enableVertexAttribArray(pos);
  gl.vertexAttribPointer(pos, 2, gl.FLOAT, false, 0, 0);
  gl.uniform1i(gl.getUniformLocation(program, 'W'), W);
  gl.uniform1i(gl.getUniformLocation(program, 'H'), H);

  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
  gl.viewport(0, 0, W / 4, H + H / 2);

  const data = new Uint8Array(W * (H + H / 2));
  return {
    /** A frame of `source` (a canvas the size W x H) at `timestamp` for `duration` (µs). */
    frame(source, timestamp, duration) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.readPixels(0, 0, W / 4, H + H / 2, gl.RGBA, gl.UNSIGNED_BYTE, data);
      return new VideoFrame(data, {
        format: 'I420',
        codedWidth: W,
        codedHeight: H,
        timestamp,
        duration,
        colorSpace: TV_BT709,
      });
    },
    close() {
      gl.deleteTexture(texture);
      gl.deleteBuffer(buffer);
      gl.deleteProgram(program);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
    },
  };
}
