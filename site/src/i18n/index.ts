import en from './en';
import fr from './fr';

export type Lang = 'en' | 'fr';
export const langs: Lang[] = ['en', 'fr'];
const strings = { en, fr };

export function t(lang: Lang) {
  return strings[lang];
}

/** A page's path under the site's base, in `lang`: `page` is '' for the home page, 'download/' or 'privacy/'. */
export function pagePath(lang: Lang, page = ''): string {
  const base = import.meta.env.BASE_URL.replace(/\/$/, '');
  return `${base}/${lang === 'en' ? '' : `${lang}/`}${page}`;
}

/** A file in public/, under the site's base. */
export function asset(path: string): string {
  return `${import.meta.env.BASE_URL.replace(/\/$/, '')}/${path}`;
}

export const repo = 'https://github.com/C2-POLY/code2form';

/** The iPhone and iPad app on the App Store; Apple opens it in the visitor's own storefront. */
export const appStore = 'https://apps.apple.com/app/id6828989392';
