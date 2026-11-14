// The organization's branding: name, logo and colors from the admin policy (`branding`).
// From one brand color per theme, derive the accent shades the UI uses, keeping text readable.

export interface Branding {
  name: string;
  /** Data URL of the logo, or null for furnish's own. */
  logo: string | null;
  accent: string | null;
  accentDark: string | null;
  /** Any theme color by name (without `--`), per theme. */
  light: Record<string, string>;
  dark: Record<string, string>;
}

type Rgb = [number, number, number];

function parse(hex: string): Rgb {
  const h = hex.slice(1);
  const full = h.length === 3 ? [...h].map((c) => c + c).join('') : h;
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16)) as Rgb;
}
const toHex = (c: Rgb) => '#' + c.map((v) => Math.round(v).toString(16).padStart(2, '0')).join('');
/** `a` moved `t` (0…1) of the way towards `b`. */
const mix = (a: Rgb, b: Rgb, t: number): Rgb => a.map((v, i) => v + (b[i] - v) * t) as Rgb;

/** WCAG relative luminance and contrast ratio. */
function luminance([r, g, b]: Rgb) {
  const lin = (v: number) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}
export function contrast(a: Rgb, b: Rgb) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const BLACK: Rgb = [0, 0, 0];
const WHITE: Rgb = [255, 255, 255];

/** Move `color` towards `target` until it reaches 4.5:1 against `background` (readable text). */
function readable(color: Rgb, background: Rgb, target: Rgb): Rgb {
  // Checked on the rounded color: that is what the page gets.
  const rounded = (c: Rgb) => parse(toHex(c));
  for (let step = 0; step <= 20; step++) {
    const c = rounded(mix(color, target, step / 20));
    if (contrast(c, background) >= 4.5) return c;
  }
  return target;
}

/** The accent shades of one theme from its brand color. */
export function accentShades(accent: string, dark: boolean): Record<string, string> {
  const a = parse(accent);
  const canvas: Rgb = dark ? parse('#0d1a1d') : parse('#f4f7f7');
  const toward = dark ? WHITE : BLACK;
  const onAccent = contrast(a, WHITE) >= contrast(a, parse('#0b1416')) ? WHITE : parse('#0b1416');
  return {
    accent: toHex(a),
    'accent-strong': toHex(mix(a, toward, dark ? 0.2 : 0.22)),
    'accent-soft': toHex(mix(a, dark ? canvas : WHITE, dark ? 0.75 : 0.85)),
    'accent-ink': toHex(readable(mix(a, toward, dark ? 0.35 : 0.22), canvas, toward)),
    'on-accent': toHex(onAccent),
  };
}

/** CSS for the branding: overrides of the theme variables in app.css, light and dark. */
export function brandingCss(b: Branding): string {
  const block = (vars: Record<string, string>) =>
    Object.entries(vars)
      .map(([name, value]) => `--${name}: ${value};`)
      .join(' ');
  const darkAccent = b.accentDark ?? (b.accent ? toHex(mix(parse(b.accent), WHITE, 0.35)) : null);
  const light = { ...(b.accent ? accentShades(b.accent, false) : {}), ...b.light };
  const dark = { ...(darkAccent ? accentShades(darkAccent, true) : {}), ...b.dark };
  // Each theme only overrides its own colors (app.css sets the dark theme in a media query too).
  return (
    `@media not all and (prefers-color-scheme: dark) { :root { ${block(light)} } }\n` +
    `@media (prefers-color-scheme: dark) { :root { ${block(dark)} } }`
  );
}

/** Apply the branding to the page. */
export function applyBranding(b: Branding) {
  document.title = b.name;
  let style = document.getElementById('branding') as HTMLStyleElement | null;
  if (!style) {
    style = document.createElement('style');
    style.id = 'branding';
    document.head.append(style);
  }
  style.textContent = brandingCss(b);
}
