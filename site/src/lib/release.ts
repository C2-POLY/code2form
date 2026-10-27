// The latest release, read from the GitHub API once per build. A build
// without network, or over the API's rate limit, still succeeds: the
// pages then link to the releases page instead.

export interface ReleaseFile {
  name: string;
  url: string;
  size: number;
  sha256?: string;
}

export interface Release {
  version: string;
  tag: string;
  date: Date;
  url: string;
  app?: ReleaseFile;
  cli?: ReleaseFile;
  sums?: ReleaseFile;
}

const API = 'https://api.github.com/repos/C2-POLY/code2form/releases/latest';
export const RELEASES = 'https://github.com/C2-POLY/code2form/releases';

interface Asset {
  name: string;
  browser_download_url: string;
  size: number;
  digest?: string | null;
}

let cached: Promise<Release | null> | undefined;

export function latestRelease(): Promise<Release | null> {
  cached ??= fetchRelease();
  return cached;
}

async function fetchRelease(): Promise<Release | null> {
  if (process.env.SITE_OFFLINE) return null;
  const headers: Record<string, string> = {
    Accept: 'application/vnd.github+json',
    'X-GitHub-Api-Version': '2022-11-28',
  };
  // In GitHub Actions, the workflow's token lifts the rate limit.
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  try {
    const res = await fetch(API, { headers, signal: AbortSignal.timeout(10_000) });
    if (!res.ok) {
      console.warn(`[release] GitHub answered ${res.status}; linking to the releases page instead.`);
      return null;
    }
    const data = (await res.json()) as {
      tag_name: string;
      published_at: string;
      html_url: string;
      assets: Asset[];
    };
    const file = (test: (name: string) => boolean): ReleaseFile | undefined => {
      const a = data.assets.find((x) => test(x.name));
      if (!a) return undefined;
      const sha256 = a.digest?.startsWith('sha256:') ? a.digest.slice(7) : undefined;
      return { name: a.name, url: a.browser_download_url, size: a.size, sha256 };
    };
    return {
      version: data.tag_name.replace(/^v/, ''),
      tag: data.tag_name,
      date: new Date(data.published_at),
      url: data.html_url,
      app: file((n) => /^Code2Form-.*\.zip$/.test(n)),
      cli: file((n) => /^c2f-.*\.zip$/.test(n)),
      sums: file((n) => n === 'SHA256SUMS.txt'),
    };
  } catch (err) {
    console.warn(`[release] Could not reach GitHub (${(err as Error).message}); linking to the releases page instead.`);
    return null;
  }
}

/** A size in megabytes (MB, 10^6 bytes, as macOS shows it), in the page's language. */
export function formatSize(bytes: number, lang: string): string {
  const mb = (bytes / 1e6).toLocaleString(lang, { maximumFractionDigits: 1 });
  return `${mb} ${lang === 'fr' ? 'Mo' : 'MB'}`;
}

export function formatDate(date: Date, lang: string): string {
  return date.toLocaleDateString(lang === 'fr' ? 'fr-FR' : 'en-GB', {
    day: 'numeric',
    month: 'long',
    year: 'numeric',
    timeZone: 'UTC',
  });
}
