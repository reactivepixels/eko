/**
 * Galaxy visualizer — GPGPU particle galaxy ported from concepts/viz-galaxy.html.
 *
 * 512×512 = 262,144 particles simulated on the GPU each frame (ping-pong FBOs),
 * rendered as additive GL_POINTS with per-particle depth-of-field bokeh.
 * Driven by AudioFeatureTracker features; no tweak panel (DEFAULTS hardcoded).
 *
 * LIVE path only — the offline whole-song pre-analysis is not ported.
 */

import type { AudioFeatures } from "../../audio/audioFeatures";
import type { Visualizer, VisualizerDef } from "./types";

// ── Tuned defaults (from the concept's baked DEFAULTS) ─────────────────────
const D = {
  motionTempo: 2.0,
  breatheAmt: 1.5,
  waveAmt: 1.5,
  spinAmt: 1.5,
  flockAmt: 1.5,
  shedAmt: 0.85,
  shedThresh: 0.55,
  formReact: 0.98,
  calm: 1.0,
  rotRate: 0.12,
  damping: 3.3,
  dofFocal: -0.19,
  dofStrength: 1.7,
  dofMaxCoc: 7.2,
  dofBreathe: 0.8,
  dofBreatheHz: 0.2,
  pointScale: 1.11,
  glowSoft: 1.0,
  centralGlow: 0.9,
  saturation: 1.0,
  brightness: 1.0,
  hueOffset: 0.0,
  hueRange: 1.0,
  lumBreathe: 0.0,
  vignette: 1.0,
};

const NB = 32;
const TEX = 512; // 512×512 = 262,144 particles
const COUNT = TEX * TEX;

// ── GLSL sources ────────────────────────────────────────────────────────────

const SIM_VS = `#version 300 es
precision highp float;
layout(location=0) in vec2 a;
void main(){ gl_Position=vec4(a,0.0,1.0); }`;

const SIM_HEAD = `#version 300 es
precision highp float;
uniform sampler2D uPos, uVel, uSpectrum;
uniform vec2 uTexSize;
uniform float uTime,uDt;
uniform float uEnergy,uLow,uMid,uHigh;
uniform float uCalm,uDamping,uMotionTempo;
uniform float uBeatPhase,uBarPhase,uBpm;
uniform float uBreatheAmt,uWaveAmt,uSpinAmt,uFlockAmt,uFormReact;
uniform float uWaveRadius;
uniform float uShed,uShedSeed,uShedAmt,uShedLife;
out vec4 outColor;

float band(float x){ return texture(uSpectrum, vec2(clamp(x,0.0,1.0),0.5)).r; }
float bandReact(float x){ return texture(uSpectrum, vec2(clamp(x,0.0,1.0),0.5)).g; }
float hash21(vec2 p){ p=fract(p*vec2(123.34,345.45)); p+=dot(p,p+34.345); return fract(p.x*p.y); }
mat2 rot(float a){ float c=cos(a),s=sin(a); return mat2(c,-s,s,c); }

float hash31(vec3 p){ p=fract(p*0.3183099+0.1); p*=17.0; return fract(p.x*p.y*p.z*(p.x+p.y+p.z)); }
float vnoise3(vec3 x){
  vec3 i=floor(x), f=fract(x); f=f*f*(3.0-2.0*f);
  float n000=hash31(i+vec3(0,0,0)), n100=hash31(i+vec3(1,0,0));
  float n010=hash31(i+vec3(0,1,0)), n110=hash31(i+vec3(1,1,0));
  float n001=hash31(i+vec3(0,0,1)), n101=hash31(i+vec3(1,0,1));
  float n011=hash31(i+vec3(0,1,1)), n111=hash31(i+vec3(1,1,1));
  return mix(mix(mix(n000,n100,f.x),mix(n010,n110,f.x),f.y),
             mix(mix(n001,n101,f.x),mix(n011,n111,f.x),f.y),f.z);
}
float fbm3(vec3 p){ float s=0.0,a=0.5; for(int i=0;i<4;i++){ s+=a*vnoise3(p); p=p*2.02+vec3(1.7,9.2,3.3); a*=0.5; } return s; }

vec3 potential(vec3 p){
  float tt=uTime*0.12;
  vec3 q=p*1.35;
  float a=fbm3(q + vec3(0.0,0.0,tt))        + 0.5*fbm3(q*2.7 + vec3(0.0,0.0,tt*1.6));
  float b=fbm3(q*1.13 + vec3(31.4,12.0,-tt*0.8)) + 0.5*fbm3(q*2.9 + vec3(11.0,4.0,-tt));
  float c=fbm3(q*0.81 + vec3(-7.0,5.0,tt*0.6))   + 0.5*fbm3(q*3.1 + vec3(-2.0,8.0,tt*0.7));
  return vec3(a,b,c);
}
vec3 curlNoise(vec3 p){
  const float e=0.13;
  vec3 dx=vec3(e,0,0), dy=vec3(0,e,0), dz=vec3(0,0,e);
  vec3 p_x0=potential(p-dx), p_x1=potential(p+dx);
  vec3 p_y0=potential(p-dy), p_y1=potential(p+dy);
  vec3 p_z0=potential(p-dz), p_z1=potential(p+dz);
  float x=(p_y1.z-p_y0.z)-(p_z1.y-p_z0.y);
  float y=(p_z1.x-p_z0.x)-(p_x1.z-p_x0.z);
  float z=(p_x1.y-p_x0.y)-(p_y1.x-p_y0.x);
  return vec3(x,y,z)/(2.0*e);
}`;

const VEL_FS =
  SIM_HEAD +
  `
float beatPulse(float p, float sharp){ return exp(-p*sharp); }
void main(){
  ivec2 c=ivec2(gl_FragCoord.xy);
  vec4 P=texelFetch(uPos,c,0);
  vec4 V=texelFetch(uVel,c,0);
  vec3 pos=P.xyz; vec3 vel=V.xyz; float seed=V.w;

  float rr = length(pos.xy)+1e-3;
  float bandIdx = clamp(rr*0.42 + (seed-0.5)*0.10, 0.0, 1.0);
  float bv = band(bandIdx);
  vec2 radial = pos.xy/rr;
  vec2 tang = vec2(-pos.y, pos.x)/rr;

  float beatP = uBeatPhase;
  float pulse = beatPulse(beatP, 5.0);
  float cadence = mix(1.0, clamp(uBpm/120.0, 0.5, 2.2), uMotionTempo);
  float dtc = uDt * cadence;

  float bassPull = uLow;
  float armSpread= uMid;
  float disperse = uHigh;
  float idealR = 0.72 + bv*0.55
               + uFormReact*( armSpread*0.7 + disperse*0.5*step(0.9,rr) - bassPull*0.45 );
  idealR = clamp(idealR, 0.25, 2.2);

  vec3 flow = curlNoise(pos*1.05 + vec3(0.0,0.0,uTime*0.045));
  float exc = bandReact(bandIdx);
  float audioFlow = 0.20 + uEnergy*0.35 + exc*1.25;
  vel += flow * audioFlow * dtc * uCalm;
  float orbit = (0.85/(0.32+rr)) * 0.62;
  vel.xy += tang * orbit * dtc * uCalm;

  bool shedding = (P.w < 0.0);
  float roll = hash21(gl_FragCoord.xy*0.071 + vec2(uShedSeed, uShedSeed*1.7));
  bool trig = (uShed > 0.0001) && (P.w > 0.0) && (rr > 1.8) && (roll < uShed*0.95);

  if(shedding || trig){
    float kick = trig ? (3.4 + uShed*3.6) : 0.0;
    vel.xy += radial * kick * uShedAmt;
    vel.xy += tang   * kick * 0.16 * uShedAmt;
    vel.xy += radial * 1.4 * dtc;
  } else {
    float trebleSel = smoothstep(0.55, 1.0, bandIdx);
    vel.xy += tang * exc * trebleSel * 0.55 * sin(uTime*7.0 + seed*40.0) * dtc;
    vel.xy += -radial * (rr - idealR) * 0.55 * dtc * uCalm;
    float barSine  = sin(uBarPhase*6.2831853);
    float beatSine = 0.5 + 0.5*sin((uBeatPhase-0.25)*6.2831853);
    vel.xy += radial * barSine * uBreatheAmt * 0.9 * dtc;
    vel += flow * beatSine * uWaveAmt * 1.1 * dtc;
    vel.xy += tang * (0.55 + 0.45*beatSine) * uSpinAmt * (0.6/(0.4+rr)) * dtc;
    vel.xy += -radial * (rr - idealR) * uFlockAmt * 0.9 * dtc;
  }

  vel.z += (-pos.z*1.6 + flow.z*0.5*uCalm)*dtc;

  float damp = (shedding||trig) ? min(uDamping, 0.55) : uDamping;
  vel *= (1.0 - uDt*damp);
  float vmax = (shedding||trig) ? 10.0 : (0.9 + (uBreatheAmt+uWaveAmt+uSpinAmt+uFlockAmt)*0.9);
  float vl=length(vel); if(vl>vmax) vel*=vmax/vl;

  outColor=vec4(vel, seed);
}`;

const POS_FS =
  SIM_HEAD +
  `
void main(){
  ivec2 c=ivec2(gl_FragCoord.xy);
  vec4 P=texelFetch(uPos,c,0);
  vec4 V=texelFetch(uVel,c,0);
  vec3 pos=P.xyz; float life=P.w; float seed=V.w;
  float rr0=length(P.xy)+1e-3;

  pos += V.xyz * uDt;

  float roll = hash21(gl_FragCoord.xy*0.071 + vec2(uShedSeed, uShedSeed*1.7));
  bool trig = (uShed > 0.0001) && (P.w > 0.0) && (rr0 > 1.8) && (roll < uShed*0.95);
  if(trig){
    life = -1.0;
  } else if(P.w < 0.0){
    life = P.w + uDt/max(0.05, uShedLife);
  } else {
    life = P.w - uDt*(0.05 + 0.045*hash21(gl_FragCoord.xy*0.013));
  }
  float dist=length(pos.xy);
  bool finishedShed = (P.w < 0.0 && life >= 0.0);
  bool diedBound    = (P.w > 0.0 && life <= 0.0);
  bool respawn = finishedShed || diedBound || (dist>4.5);
  if(respawn){
    vec2 h=gl_FragCoord.xy*0.0179 + uTime*0.137 + seed*7.0;
    float bulge=hash21(h+4.4);
    if(bulge<0.22){
      float r=pow(hash21(h+1.3),1.6)*0.55;
      float a=hash21(h+5.7)*6.2831853;
      float z=(hash21(h+9.1)-0.5)*0.3*exp(-r);
      pos=vec3(cos(a)*r, sin(a)*r, z);
    } else {
      float armSel=floor(hash21(h+3.1)*3.0);
      float arm=armSel*(6.2831853/3.0);
      float r=pow(hash21(h+1.31),0.5)*1.85 + 0.18;
      float twist=2.0;
      float a = arm + r*twist + (hash21(h+5.7)-0.5)*1.35;
      float z=(hash21(h+9.1)-0.5)*0.34*exp(-r*0.5);
      pos=vec3(cos(a)*r, sin(a)*r, z);
    }
    life=0.7+0.45*hash21(h+2.2);
  }
  outColor=vec4(pos, life);
}`;

const RENDER_VS = `#version 300 es
precision highp float;
uniform sampler2D uPos, uVel, uSpectrum;
uniform vec2 uTexSize, uRes;
uniform float uTime,uDpr,uEnergy,uPalette,uLow,uHigh,uTheme;
uniform float uCalm,uFocal,uDofK,uDofMax;
uniform float uRotRate,uPointScale,uSat,uBright,uHueOff,uHueRange;
uniform float uBeatPhase,uBarPhase,uBpm,uSpinAmt,uMotionTempo,uLumBreathe,uBreatheAmt;
uniform float uFieldAngle;
uniform float uPump;
uniform vec3 uAccent;
out vec3 vCol;
out float vGlow;
out float vCoc;
flat out int vDark;

vec3 hsv2rgb(vec3 c){ vec4 K=vec4(1.,2./3.,1./3.,3.); vec3 p=abs(fract(c.xxx+K.xyz)*6.-K.www); return c.z*mix(K.xxx,clamp(p-K.xxx,0.,1.),c.y); }
float band(float x){ return texture(uSpectrum, vec2(clamp(x,0.0,1.0),0.5)).r; }
float bandReact(float x){ return texture(uSpectrum, vec2(clamp(x,0.0,1.0),0.5)).g; }
float hash21(vec2 p){ p=fract(p*vec2(123.34,345.45)); p+=dot(p,p+34.345); return fract(p.x*p.y); }
mat2 rot(float a){ float c=cos(a),s=sin(a); return mat2(c,-s,s,c); }

void main(){
  int id=gl_VertexID;
  int tw=int(uTexSize.x);
  ivec2 c=ivec2(id % tw, id / tw);
  vec4 P=texelFetch(uPos,c,0);
  vec4 V=texelFetch(uVel,c,0);
  vec3 pos=P.xyz; float life=P.w; float seed=V.w;

  float tilt=0.42;
  float ang = uFieldAngle;
  pos.xy = rot(ang) * pos.xy;
  {
    float rr2 = length(pos.xy);
    vec2 rdir = pos.xy / max(rr2, 1e-3);
    float ang2 = atan(pos.y, pos.x);
    float front = exp(-abs(rr2 - uPump*1.7) * 1.3);
    float lobes = 0.82 + 0.18*sin(ang2*3.0 + uFieldAngle*1.5);
    float breath = (uPump*0.45 + front*0.70) * lobes;
    pos.xy += rdir * breath * uBreatheAmt * 0.5;
    pos.z  *= (1.0 + uPump * uBreatheAmt * 0.18);
  }
  vec3 wp = pos;
  float cy=cos(tilt), sy=sin(tilt);
  wp = vec3(wp.x, wp.y*cy - wp.z*sy, wp.y*sy + wp.z*cy);

  float camZ = 4.2;
  float zc = camZ - wp.z;
  vec2 proj = wp.xy / max(zc*0.5, 0.2);
  float aspect = uRes.x/uRes.y;
  vec2 clip = vec2(proj.x/aspect, proj.y);
  gl_Position=vec4(clip*0.66, 0.0, 1.0);

  float viewDepth = wp.z;
  float defocus = abs(viewDepth - uFocal);
  float coc = clamp(defocus * uDofK, 0.0, uDofMax);
  vCoc = clamp(coc/uDofMax, 0.0, 1.0);

  vec2 uvp=vec2(c)/uTexSize;
  float bandIdx=fract(uvp.x*3.0 + uvp.y*0.37 + seed*0.5);
  float bv=band(bandIdx);
  float rad=length(pos.xy);
  float sparkle=hash21(uvp+seed*3.1);

  float depthFade = clamp((5.2 - zc)/4.0, 0.25, 1.0);
  float baseSize = (1.7 + uEnergy*0.35) * uDpr * depthFade * uPointScale;
  float ps = baseSize * (1.0 + coc);
  gl_PointSize = clamp(ps, 1.0, 34.0*uDpr);

  float hot=clamp(bv*0.55 + (1.0-clamp(rad*0.7,0.0,1.0))*0.35 + sparkle*0.30, 0.0, 1.0);

  if(uPalette>0.5){
    float ang3=atan(pos.y,pos.x);
    float hue=fract(0.74 + uHueOff - (rad*0.34 + bandIdx*0.10)*uHueRange + ang3*0.05 + seed*0.03);
    float sat=clamp((0.92 - hot*0.18 - rad*0.06)*uSat, 0.0, 1.0);
    float val=0.5 + hot*0.55 + bv*0.35;
    vec3 col=hsv2rgb(vec3(hue,sat,val));
    col=mix(col, vec3(1.0,0.96,0.9), smoothstep(0.85,1.0,hot)*0.5);
    col=mix(col, uAccent*1.25, smoothstep(0.45,0.9,hot)*0.30);
    vCol=col;
  } else {
    float l=0.35 + hot*0.85 + bv*0.4;
    vec3 col=vec3(l)*vec3(0.9,0.93,1.0);
    col=mix(col, uAccent*1.3, smoothstep(0.7,1.0,hot)*0.5);
    vCol=col;
  }

  float bright = (0.26 + hot*0.5 + bv*0.3) * depthFade * uBright;
  bright *= (0.7 + 0.6*sparkle);
  bright *= (1.0 + uLumBreathe*0.18*sin(uBarPhase*6.2831853));
  bright *= smoothstep(0.0,0.18,life) * clamp(life*1.5,0.0,1.0);
  float area = (1.0 + coc);
  bright /= (0.5 + area*0.5);
  bright *= (1.0 + (1.0 - vCoc)*(1.0 - vCoc)*1.6);

  if(uTheme<0.5){
    vec3 ink = mix(vec3(0.05,0.055,0.07), vec3(0.16,0.17,0.20), sparkle*0.5);
    ink = mix(ink, uAccent*0.55, smoothstep(0.80,1.0,hot)*0.30);
    vCol = ink;
    bright = clamp(bright*1.35, 0.0, 0.7);
  }
  vDark = (uTheme<0.5)?1:0;
  if(life < 0.0){
    float shedFade = clamp(-life, 0.0, 1.0);
    bright *= shedFade * 1.5;
  }
  vGlow=bright;
}`;

const RENDER_FS = `#version 300 es
precision highp float;
in vec3 vCol; in float vGlow;
in float vCoc;
flat in int vDark;
uniform float uGlowSoft;
out vec4 fragColor;
void main(){
  vec2 d=gl_PointCoord-0.5;
  float r=length(d);
  if(r>0.5){ discard; }
  float steep = mix(20.0, 3.2, vCoc) / max(uGlowSoft, 0.2);
  float a = exp(-r*r*steep);
  float core = (1.0 - vCoc);
  a += core*core * exp(-r*r*70.0) * 0.9;
  a = clamp(a, 0.0, 1.0);
  if(vDark==1){
    vec3 c=vCol*vGlow*a;
    fragColor=vec4(c, a*vGlow);
  } else {
    fragColor=vec4(vCol, clamp(a*vGlow*2.1, 0.0, 0.92));
  }
}`;

const BG_VS = `#version 300 es
precision highp float;
layout(location=0) in vec2 a; out vec2 vUv;
void main(){ vUv=a*0.5+0.5; gl_Position=vec4(a,0.0,1.0); }`;

const BG_FS = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 fragColor;
uniform vec2 uRes; uniform vec3 uBg,uAccent; uniform float uEnergy,uLow,uTheme,uPalette,uVignette,uCentralGlow;
void main(){
  vec2 p=(gl_FragCoord.xy-0.5*uRes)/min(uRes.x,uRes.y);
  float r=length(p);
  vec3 bg=uBg;
  float core=exp(-r*r*5.5)*(0.10+uEnergy*0.18)*uCentralGlow;
  vec3 glowCol = uPalette>0.5 ? mix(vec3(0.20,0.10,0.30),uAccent,0.5) : uAccent;
  bg += glowCol*core;
  bg += glowCol*exp(-r*r*1.6)*0.03*(0.5+uEnergy)*uCentralGlow;
  bg *= mix(1.0, smoothstep(1.35,0.2,r), clamp(uVignette,0.0,2.0));
  if(uTheme<0.5){ bg = uBg; }
  fragColor=vec4(bg,1.0);
}`;

// ── Helpers ─────────────────────────────────────────────────────────────────

function hash(x: number): number {
  const s = Math.sin(x) * 43758.5453;
  return s - Math.floor(s);
}

function sh(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader | null {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    console.error("Shader compile error:\n" + gl.getShaderInfoLog(s));
    return null;
  }
  return s;
}

function buildProg(gl: WebGL2RenderingContext, vsrc: string, fsrc: string): WebGLProgram | null {
  const v = sh(gl, gl.VERTEX_SHADER, vsrc);
  const f = sh(gl, gl.FRAGMENT_SHADER, fsrc);
  if (!v || !f) return null;
  const p = gl.createProgram()!;
  gl.attachShader(p, v);
  gl.attachShader(p, f);
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    console.error("Program link error:\n" + gl.getProgramInfoLog(p));
    return null;
  }
  return p;
}

/** Cached uniform location getter. */
function makeU(gl: WebGL2RenderingContext, p: WebGLProgram) {
  const cache: Record<string, WebGLUniformLocation | null> = {};
  return (name: string): WebGLUniformLocation | null => {
    if (!(name in cache)) cache[name] = gl.getUniformLocation(p, name);
    return cache[name];
  };
}

// ── Galaxy Visualizer ────────────────────────────────────────────────────────

class GalaxyVisualizer implements Visualizer {
  private readonly gl: WebGL2RenderingContext;
  private readonly _theme: () => "dark" | "light";
  private readonly _accent: () => [number, number, number];

  // GL objects
  private readonly _quad: WebGLVertexArrayObject;
  private readonly _emptyVAO: WebGLVertexArrayObject;
  private readonly _specTex: WebGLTexture;
  private readonly _specPix = new Uint8Array(NB * 4);

  // Float texture support
  private readonly _floatOk: boolean;

  private _posA: WebGLTexture;
  private _posB: WebGLTexture;
  private _velA: WebGLTexture;
  private _velB: WebGLTexture;
  private readonly _fboS: WebGLFramebuffer;

  private readonly _progBg: WebGLProgram;
  private readonly _progRender: WebGLProgram;
  private readonly _progPos: WebGLProgram;
  private readonly _progVel: WebGLProgram;
  private readonly _uBg: (n: string) => WebGLUniformLocation | null;
  private readonly _uR: (n: string) => WebGLUniformLocation | null;
  private readonly _uP: (n: string) => WebGLUniformLocation | null;
  private readonly _uV: (n: string) => WebGLUniformLocation | null;

  // Sizing
  private _w = 1;
  private _h = 1;
  private _dpr = 1;

  constructor(
    gl: WebGL2RenderingContext,
    opts: {
      theme: () => "dark" | "light";
      accent: () => [number, number, number];
    },
  ) {
    this.gl = gl;
    this._theme = opts.theme;
    this._accent = opts.accent;

    // Float texture support
    const extCBF = gl.getExtension("EXT_color_buffer_float");
    gl.getExtension("OES_texture_float_linear");
    this._floatOk = Boolean(extCBF);

    // Quad VAO for fullscreen triangle passes
    this._quad = gl.createVertexArray()!;
    gl.bindVertexArray(this._quad);
    const qb = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, qb);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);

    // Empty VAO for the particle draw (gl_VertexID only)
    this._emptyVAO = gl.createVertexArray()!;

    // Spectrum texture 32×1
    this._specTex = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, this._specTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, NB, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, this._specPix);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

    // Seed particle state
    const { posA, posB, velA, velB } = this._seedState();
    this._posA = posA;
    this._posB = posB;
    this._velA = velA;
    this._velB = velB;

    this._fboS = gl.createFramebuffer()!;

    // Build shader programs
    this._progBg = buildProg(gl, BG_VS, BG_FS)!;
    this._progRender = buildProg(gl, RENDER_VS, RENDER_FS)!;
    this._progVel = buildProg(gl, SIM_VS, VEL_FS)!;
    this._progPos = buildProg(gl, SIM_VS, POS_FS)!;

    this._uBg = makeU(gl, this._progBg);
    this._uR = makeU(gl, this._progRender);
    this._uV = makeU(gl, this._progVel);
    this._uP = makeU(gl, this._progPos);
  }

  resize(w: number, h: number, dpr: number): void {
    this._w = w;
    this._h = h;
    this._dpr = dpr;
  }

  frame(f: AudioFeatures, dtMs: number): void {
    const dt = Math.min(dtMs / 1000, 0.05);
    const gl = this.gl;

    this._uploadSpec(f);
    this._simStep(f, dt);
    this._drawScreen(f, dt);

    const e = gl.getError();
    if (e !== gl.NO_ERROR) {
      console.warn("Galaxy GL error:", e);
    }
  }

  dispose(): void {
    const gl = this.gl;
    gl.deleteTexture(this._specTex);
    gl.deleteTexture(this._posA);
    gl.deleteTexture(this._posB);
    gl.deleteTexture(this._velA);
    gl.deleteTexture(this._velB);
    gl.deleteFramebuffer(this._fboS);
    gl.deleteProgram(this._progBg);
    gl.deleteProgram(this._progRender);
    gl.deleteProgram(this._progVel);
    gl.deleteProgram(this._progPos);
    gl.deleteVertexArray(this._quad);
    gl.deleteVertexArray(this._emptyVAO);
  }

  // ── Private ──────────────────────────────────────────────────────────────

  private _makeStateTex(data: Float32Array): WebGLTexture {
    const gl = this.gl;
    const t = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, t);
    if (this._floatOk) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, TEX, TEX, 0, gl.RGBA, gl.FLOAT, data);
    } else {
      // fallback: encode as RGBA8 (reduced precision but still functional)
      const u8 = new Uint8Array(data.length);
      for (let i = 0; i < data.length; i++)
        u8[i] = Math.max(0, Math.min(255, (data[i] * 0.5 + 0.5) * 255));
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, TEX, TEX, 0, gl.RGBA, gl.UNSIGNED_BYTE, u8);
    }
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  }

  private _seedState() {
    const pos = new Float32Array(COUNT * 4);
    const vel = new Float32Array(COUNT * 4);
    for (let i = 0; i < COUNT; i++) {
      let px: number, py: number, pz: number;
      if (hash(i * 4.4 + 0.5) < 0.22) {
        const r = Math.pow(hash(i * 0.131 + 1.0), 1.6) * 0.55;
        const a = hash(i * 7.13 + 2.0) * Math.PI * 2.0;
        px = Math.cos(a) * r;
        py = Math.sin(a) * r;
        pz = (hash(i * 3.77 + 5.0) - 0.5) * 0.3 * Math.exp(-r);
      } else {
        const arm = Math.floor(hash(i * 3.1 + 0.5) * 3.0) * ((Math.PI * 2.0) / 3.0);
        const r = Math.pow(hash(i * 0.131 + 1.0), 0.5) * 1.85 + 0.18;
        const a = arm + r * 2.0 + (hash(i * 7.13 + 2.0) - 0.5) * 1.35;
        px = Math.cos(a) * r;
        py = Math.sin(a) * r;
        pz = (hash(i * 3.77 + 5.0) - 0.5) * 0.34 * Math.exp(-r * 0.5);
      }
      pos[i * 4 + 0] = px;
      pos[i * 4 + 1] = py;
      pos[i * 4 + 2] = pz;
      pos[i * 4 + 3] = hash(i * 1.7 + 9.0);
      vel[i * 4 + 0] = 0;
      vel[i * 4 + 1] = 0;
      vel[i * 4 + 2] = 0;
      vel[i * 4 + 3] = hash(i * 0.911 + 13.0);
    }
    return {
      posA: this._makeStateTex(pos),
      posB: this._makeStateTex(pos),
      velA: this._makeStateTex(vel),
      velB: this._makeStateTex(vel),
    };
  }

  private _uploadSpec(f: AudioFeatures): void {
    const gl = this.gl;
    const px = this._specPix;
    for (let b = 0; b < NB; b++) {
      const v = Math.max(0, Math.min(255, f.bands[b] * 255)) | 0;
      const rx = Math.max(0, Math.min(255, f.react[b] * 255)) | 0;
      px[b * 4 + 0] = v;
      px[b * 4 + 1] = rx;
      px[b * 4 + 2] = v;
      px[b * 4 + 3] = 255;
    }
    gl.bindTexture(gl.TEXTURE_2D, this._specTex);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, NB, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
  }

  private _simStep(f: AudioFeatures, dt: number): void {
    const gl = this.gl;
    gl.disable(gl.BLEND);
    gl.bindVertexArray(this._quad);
    gl.viewport(0, 0, TEX, TEX);

    // --- velocity pass ---
    gl.useProgram(this._progVel);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this._fboS);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this._velB, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this._posA);
    gl.uniform1i(this._uV("uPos"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this._velA);
    gl.uniform1i(this._uV("uVel"), 1);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, this._specTex);
    gl.uniform1i(this._uV("uSpectrum"), 2);
    gl.uniform2f(this._uV("uTexSize"), TEX, TEX);
    gl.uniform1f(this._uV("uTime"), f.t);
    gl.uniform1f(this._uV("uDt"), dt);
    gl.uniform1f(this._uV("uEnergy"), f.smEnergy);
    gl.uniform1f(this._uV("uLow"), f.smLow);
    gl.uniform1f(this._uV("uMid"), f.smMid);
    gl.uniform1f(this._uV("uHigh"), f.smHigh);
    gl.uniform1f(this._uV("uCalm"), D.calm);
    gl.uniform1f(this._uV("uDamping"), D.damping);
    gl.uniform1f(this._uV("uMotionTempo"), D.motionTempo);
    gl.uniform1f(this._uV("uBeatPhase"), f.beatPhase);
    gl.uniform1f(this._uV("uBarPhase"), f.barPhase);
    gl.uniform1f(this._uV("uBpm"), f.bpm);
    gl.uniform1f(this._uV("uBreatheAmt"), D.breatheAmt);
    gl.uniform1f(this._uV("uWaveAmt"), D.waveAmt);
    gl.uniform1f(this._uV("uSpinAmt"), D.spinAmt);
    gl.uniform1f(this._uV("uFlockAmt"), D.flockAmt);
    gl.uniform1f(this._uV("uFormReact"), D.formReact);
    gl.uniform1f(this._uV("uWaveRadius"), 0);
    gl.uniform1f(this._uV("uShed"), f.shedImpulse);
    gl.uniform1f(this._uV("uShedSeed"), f.shedSeedJS);
    gl.uniform1f(this._uV("uShedAmt"), D.shedAmt);
    gl.uniform1f(this._uV("uShedLife"), 1.9);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // --- position pass ---
    gl.useProgram(this._progPos);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this._posB, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this._posA);
    gl.uniform1i(this._uP("uPos"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this._velB);
    gl.uniform1i(this._uP("uVel"), 1);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, this._specTex);
    gl.uniform1i(this._uP("uSpectrum"), 2);
    gl.uniform2f(this._uP("uTexSize"), TEX, TEX);
    gl.uniform1f(this._uP("uTime"), f.t);
    gl.uniform1f(this._uP("uDt"), dt);
    gl.uniform1f(this._uP("uShed"), f.shedImpulse);
    gl.uniform1f(this._uP("uShedSeed"), f.shedSeedJS);
    gl.uniform1f(this._uP("uShedAmt"), D.shedAmt);
    gl.uniform1f(this._uP("uShedLife"), 1.9);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);

    // swap ping-pong
    let tmp = this._posA;
    this._posA = this._posB;
    this._posB = tmp;
    tmp = this._velA;
    this._velA = this._velB;
    this._velB = tmp;
  }

  private _drawScreen(f: AudioFeatures, dt: number): void {
    const gl = this.gl;
    const theme = this._theme();
    const accent = this._accent();
    const isDark = theme === "dark";
    const bgR = isDark ? 8 / 255 : 244 / 255;
    const bgG = isDark ? 9 / 255 : 245 / 255;
    const bgB = isDark ? 11 / 255 : 247 / 255;
    const palette = isDark ? 1.0 : 0.0;

    const W = this._w * this._dpr;
    const H = this._h * this._dpr;

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, W, H);

    // --- background ---
    gl.disable(gl.BLEND);
    gl.useProgram(this._progBg);
    gl.bindVertexArray(this._quad);
    gl.uniform2f(this._uBg("uRes"), W, H);
    gl.uniform3f(this._uBg("uBg"), bgR, bgG, bgB);
    gl.uniform3f(this._uBg("uAccent"), accent[0], accent[1], accent[2]);
    gl.uniform1f(this._uBg("uEnergy"), f.smEnergy);
    gl.uniform1f(this._uBg("uLow"), f.smLow);
    gl.uniform1f(this._uBg("uTheme"), isDark ? 1.0 : 0.0);
    gl.uniform1f(this._uBg("uPalette"), palette);
    gl.uniform1f(this._uBg("uVignette"), D.vignette);
    gl.uniform1f(this._uBg("uCentralGlow"), D.centralGlow);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // --- particles ---
    gl.enable(gl.BLEND);
    if (isDark) {
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE); // additive → luminous density
    } else {
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA); // ink dust on light
    }
    gl.useProgram(this._progRender);
    gl.bindVertexArray(this._emptyVAO);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this._posA);
    gl.uniform1i(this._uR("uPos"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this._velA);
    gl.uniform1i(this._uR("uVel"), 1);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, this._specTex);
    gl.uniform1i(this._uR("uSpectrum"), 2);
    gl.uniform2f(this._uR("uTexSize"), TEX, TEX);
    gl.uniform2f(this._uR("uRes"), W, H);
    gl.uniform1f(this._uR("uTime"), f.t);
    gl.uniform1f(this._uR("uDpr"), this._dpr);
    gl.uniform1f(this._uR("uEnergy"), f.smEnergy);
    gl.uniform1f(this._uR("uLow"), f.smLow);
    gl.uniform1f(this._uR("uHigh"), f.smHigh);
    gl.uniform1f(this._uR("uPalette"), palette);
    gl.uniform1f(this._uR("uTheme"), isDark ? 1.0 : 0.0);
    gl.uniform1f(this._uR("uCalm"), D.calm);
    gl.uniform1f(this._uR("uRotRate"), D.rotRate);
    gl.uniform1f(this._uR("uPointScale"), D.pointScale);
    gl.uniform1f(this._uR("uGlowSoft"), D.glowSoft);
    gl.uniform1f(this._uR("uSat"), D.saturation);
    gl.uniform1f(this._uR("uBright"), D.brightness);
    gl.uniform1f(this._uR("uHueOff"), D.hueOffset);
    gl.uniform1f(this._uR("uHueRange"), D.hueRange);
    gl.uniform1f(this._uR("uBeatPhase"), f.beatPhase);
    gl.uniform1f(this._uR("uBarPhase"), f.barPhase);
    gl.uniform1f(this._uR("uBpm"), f.bpm);
    gl.uniform1f(this._uR("uSpinAmt"), D.spinAmt);
    gl.uniform1f(this._uR("uMotionTempo"), D.motionTempo);
    gl.uniform1f(this._uR("uLumBreathe"), D.lumBreathe);
    gl.uniform1f(this._uR("uBreatheAmt"), D.breatheAmt);
    gl.uniform1f(this._uR("uFieldAngle"), f.fieldAngle);
    gl.uniform1f(this._uR("uPump"), f.pump);
    // DoF: slow focal breathe
    const focal =
      D.dofFocal +
      Math.sin(f.t * D.dofBreatheHz * 6.2831853) * D.dofBreathe +
      (f.smEnergy - 0.15) * 0.3;
    gl.uniform1f(this._uR("uFocal"), focal);
    gl.uniform1f(this._uR("uDofK"), D.dofStrength);
    gl.uniform1f(this._uR("uDofMax"), D.dofMaxCoc);
    gl.uniform3f(this._uR("uAccent"), accent[0], accent[1], accent[2]);
    gl.drawArrays(gl.POINTS, 0, COUNT);
    gl.disable(gl.BLEND);
    gl.bindVertexArray(null);
    void dt; // dt used implicitly via feature tracker; suppress unused-var
  }
}

// ── VisualizerDef export ────────────────────────────────────────────────────

export const galaxy: VisualizerDef = {
  id: "galaxy",
  label: "Particle Galaxy",
  create(gl, opts) {
    return new GalaxyVisualizer(gl, opts);
  },
};
