// The site's motion: smooth scrolling with Lenis, driven by GSAP's ticker so
// that ScrollTrigger reads the same scroll position, and the reveals the
// markup asks for:
//
//   data-intro           its children rise in one after the other on load
//   data-reveal          rises in once it comes into view
//   data-reveal-stagger  its children rise in one after the other in view
//   data-parallax="n"    drifts n pixels against the scroll while in view
//
// With reduced motion none of this runs and the page stays as the HTML has
// it. The inline script in Base.astro hides [data-intro] until this runs
// (`html.motion`), and shows it again if this never does.
import Lenis from 'lenis';
import 'lenis/dist/lenis.css';
import gsap from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';

gsap.registerPlugin(ScrollTrigger);

const root = document.documentElement;

export function startMotion() {
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
    root.classList.remove('motion');
    return;
  }

  // Anchor links scroll smoothly; Lenis keeps them below the sticky header
  // by reading the page's scroll-padding-top.
  const lenis = new Lenis({ autoRaf: false, lerp: 0.12, anchors: true });
  lenis.on('scroll', ScrollTrigger.update);
  gsap.ticker.add((time) => lenis.raf(time * 1000));
  gsap.ticker.lagSmoothing(0);

  const rise = { y: 28, autoAlpha: 0, duration: 0.8, ease: 'power3.out' };

  // One stagger across every intro group, in the order of the page.
  const intro = [...document.querySelectorAll<HTMLElement>('[data-intro]')].flatMap((g) => [...g.children]);
  if (intro.length) gsap.from(intro, { ...rise, y: 20, stagger: 0.08, delay: 0.05 });
  root.classList.add('motion-ready');

  for (const el of document.querySelectorAll<HTMLElement>('[data-reveal]')) {
    gsap.from(el, { ...rise, scrollTrigger: { trigger: el, start: 'top 88%', once: true } });
  }

  for (const group of document.querySelectorAll<HTMLElement>('[data-reveal-stagger]')) {
    gsap.from(group.children, {
      ...rise,
      stagger: 0.1,
      scrollTrigger: { trigger: group, start: 'top 85%', once: true },
    });
  }

  for (const el of document.querySelectorAll<HTMLElement>('[data-parallax]')) {
    const distance = Number(el.dataset.parallax) || 24;
    gsap.fromTo(
      el,
      { y: distance },
      { y: -distance, ease: 'none', scrollTrigger: { trigger: el, start: 'top bottom', end: 'bottom top', scrub: true } },
    );
  }

  // Images and the scroll story change the page's height once they load.
  addEventListener('load', () => ScrollTrigger.refresh());
}
