// @ts-check
import { defineConfig } from 'astro/config';

// Published on GitHub Pages at https://c2-poly.github.io/code2form/. For a
// custom domain later, set SITE_URL to it and SITE_BASE to "/" (and add
// public/CNAME); nothing else names the domain.
const site = process.env.SITE_URL ?? 'https://c2-poly.github.io';
const base = process.env.SITE_BASE ?? '/code2form';

export default defineConfig({
  site,
  base,
  trailingSlash: 'always',
  i18n: {
    locales: ['en', 'fr'],
    defaultLocale: 'en',
    routing: { prefixDefaultLocale: false },
  },
  build: {
    // One stylesheet per page, small enough to inline: no render-blocking request.
    inlineStylesheets: 'always',
  },
});
