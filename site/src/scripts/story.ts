// The scroll story's script: frame sequences follow the scroll, the
// agent's edit and the counters play with it. It turns the story on
// (`html.story`) only when motion is welcome; otherwise the page stays as
// the HTML has it, final frames and numbers included.

const clamp = (t: number) => Math.min(1, Math.max(0, t));
const band = (t: number, a: number, b: number) => clamp((t - a) / (b - a));
const ease = (t: number) => t * t * (3 - 2 * t);

interface Sequence {
  section: HTMLElement;
  img: HTMLImageElement;
  frames: HTMLImageElement[];
  loaded: boolean[];
  shown: number;
  kind: string;
}

export function startStory() {
  const root = document.querySelector<HTMLElement>('[data-story]');
  if (!root) return;
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  document.documentElement.classList.add('story');

  const lang = document.documentElement.lang;
  const base = root.dataset.framesBase ?? '';
  const scrollTimelines = CSS.supports('animation-timeline: view()');

  const sequences: Sequence[] = [...root.querySelectorAll<HTMLElement>('[data-seq]')].map((section) => {
    const kind = section.dataset.seq ?? '';
    const count = Number(section.dataset.count);
    const img = section.querySelector<HTMLImageElement>('.frame')!;
    return { section, img, frames: [], loaded: new Array(count).fill(false), shown: -1, kind };
  });

  // Each sequence's frames load once it comes within a screen of view.
  const load = (seq: Sequence) => {
    if (seq.frames.length) return;
    seq.frames = seq.loaded.map((_, i) => {
      const frame = new Image();
      frame.decoding = 'async';
      frame.src = `${base}${seq.kind}-${String(i).padStart(2, '0')}.webp`;
      frame.decode().then(
        () => {
          seq.loaded[i] = true;
          // A frame that just arrived shows without waiting for a scroll.
          schedule();
        },
        () => {},
      );
      return frame;
    });
  };
  const near = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const seq = sequences.find((s) => s.section === entry.target);
        if (seq) load(seq);
        near.unobserve(entry.target);
      }
    },
    { rootMargin: '100% 0px' },
  );
  sequences.forEach((s) => near.observe(s.section));

  const edit = root.querySelector<HTMLElement>('[data-edit]');
  const domeValue = edit?.querySelector<HTMLElement>('[data-dome-value]');
  const height = edit?.querySelector<HTMLElement>('[data-height]');
  const report = edit?.querySelector<HTMLElement>('[data-report]');
  const numbers = root.querySelector<HTMLElement>('[data-numbers]');
  const counters = numbers ? [...numbers.querySelectorAll<HTMLElement>('[data-count-to]')] : [];
  const progressBar = document.querySelector<HTMLElement>('.progress-bar');

  const format = (n: number, digits: number) =>
    n.toLocaleString(lang, { minimumFractionDigits: digits, maximumFractionDigits: digits });

  // How far through a tall section the sticky stage is: 0 when its top
  // reaches the top of the window, 1 when its bottom reaches the bottom
  // (the range CSS calls `contain`).
  const through = (el: Element, vh: number) => {
    const r = el.getBoundingClientRect();
    return clamp(-r.top / Math.max(1, r.height - vh));
  };
  const visible = (el: Element, vh: number) => {
    const r = el.getBoundingClientRect();
    return r.bottom > 0 && r.top < vh;
  };

  const showFrame = (seq: Sequence, t: number) => {
    const n = seq.loaded.length;
    let i = Math.min(n - 1, Math.round(t * (n - 1)));
    // A frame still loading: keep the nearest one loaded before it.
    while (i > 0 && !seq.loaded[i]) i--;
    if (!seq.loaded[i] || i === seq.shown) return;
    seq.shown = i;
    seq.img.src = seq.frames[i].src;
  };

  let lastReport = '';
  const update = () => {
    pending = 0;
    const vh = window.innerHeight;

    for (const seq of sequences) {
      if (!visible(seq.section, vh)) continue;
      const t = through(seq.section, vh);
      if (seq.kind === 'turn') {
        showFrame(seq, t);
        seq.section.dataset.step = String(t < 0.335 ? 0 : t < 0.665 ? 1 : 2);
        if (!scrollTimelines) seq.section.style.setProperty('--zoom', String(ease(band(t, 0, 0.3))));
      } else {
        showFrame(seq, ease(band(t, 0.05, 0.85)));
        const shown = [0.35, 0.47, 0.59, 0.71].filter((a) => t >= a + 0.06).length;
        seq.section.dataset.shown = String(shown);
      }
    }

    if (edit && visible(edit, vh)) {
      const t = through(edit, vh);
      const grow = ease(band(t, 0.25, 0.75));
      const dome = 26 + 8 * grow;
      edit.style.setProperty('--dome-mm', dome.toFixed(2));
      edit.style.setProperty('--tint', t > 0.2 ? (1 - band(t, 0.8, 1)).toFixed(3) : '0');
      if (domeValue) domeValue.textContent = String(Math.round(dome));
      if (height) height.textContent = format(55.99 + 7.99 * grow, 2);
      if (report) {
        const state = t > 0.75 ? 'done' : t > 0.25 ? 'working' : 'start';
        if (state !== lastReport) {
          lastReport = state;
          report.textContent = report.dataset[state] ?? '';
          report.classList.toggle('done', state === 'done');
        }
      }
    }

    if (numbers && visible(numbers, vh)) {
      const r = numbers.getBoundingClientRect();
      const t = ease(clamp((vh - r.top) / (vh * 0.6)));
      for (const c of counters) {
        const from = Number(c.dataset.countFrom);
        const to = Number(c.dataset.countTo);
        c.textContent = format(from + (to - from) * t, Number(c.dataset.digits));
      }
    }

    if (progressBar && !CSS.supports('animation-timeline: scroll()')) {
      const doc = document.documentElement;
      progressBar.style.setProperty('--page', String(clamp(scrollY / Math.max(1, doc.scrollHeight - vh))));
    }
  };

  let pending = 0;
  const schedule = () => {
    if (!pending) pending = requestAnimationFrame(update);
  };
  addEventListener('scroll', schedule, { passive: true });
  addEventListener('resize', schedule);
  update();
}
