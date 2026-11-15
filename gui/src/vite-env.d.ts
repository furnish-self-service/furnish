// Vite imports images as URLs.
declare module '*.svg' {
  const src: string;
  export default src;
}

// Stylesheets are imported for their effect.
declare module '*.css';
