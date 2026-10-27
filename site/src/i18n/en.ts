// Every string the English pages show. fr.ts has the same keys.
const en = {
  meta: {
    title: 'Code2Form: parametric 3D design as code, for your 3D printer',
    description:
      'Free and open source. Write a part in a few lines, watch it render as you type, check it and export STL or 3MF for your slicer. Opens your .scad files as they are.',
    downloadTitle: 'Download Code2Form',
    downloadDescription:
      'Download Code2Form for macOS (Apple silicon and Intel), install its MCP server for Claude Code, or build it from source on Windows and Linux.',
  },
  a11y: {
    skip: 'Skip to content',
    mainNav: 'Main',
    theme: 'Switch between the dark and light themes',
    language: 'Language',
    progress: 'How far down the page you are',
  },
  nav: {
    overview: 'Overview',
    examples: 'Examples',
    docs: 'Docs',
    download: 'Download',
    github: 'GitHub',
  },
  hero: {
    badge: 'free and open source',
    titleStart: 'Describe a part in a few lines of code.',
    titleAccent: 'Print it tonight.',
    lead:
      'Code2Form renders as you type, weighs the part and checks its gears and wiring before the first layer, then exports STL and 3MF for your slicer. Your .scad files open as they are.',
    downloadMac: 'Download for macOS',
    printExample: 'Print an example',
    shotAlt:
      'The Code2Form window: a night light in the 3D view, and the agent’s chat floating over it, showing the line of code it changed',
    shotCaption: 'The agent’s chat, floating over the view.',
  },
  ways: {
    title: 'One model, three ways in',
    soon: 'Next release',
    code: {
      label: 'Code',
      title: 'In code',
      text:
        'Short programs in the .scad language. Numbers you mark in a comment get sliders, and the view follows every keystroke.',
    },
    mouse: {
      title: 'With the mouse',
      tools: ['Shapes', 'Move', 'Sketch', 'Extrude'],
      text:
        'Add shapes from a toolbar, drag handles to move them, draw a sketch and extrude it in the view. Each gesture is written into the code, one undo step at a time.',
    },
    agent: {
      ask: 'Make the dome 8 mm taller.',
      title: 'By asking',
      text:
        'A chat floats over the view and edits the open model live, through your Claude subscription with Claude Code, or an API key.',
    },
  },
  story: {
    intro: 'The watch winder, designed in Code2Form',
    titleStart: 'Thought through.',
    titleAccent: 'Then printed.',
    lead: 'Scroll to turn it, take it apart, and see what the plan changed.',
    turnAlt: 'The watch winder: a tilted drum on a printed frame, turning as the page scrolls',
    captions: [
      'A watch winder for a bedside table.',
      'The drum turns on three 608 bearings, not on the motor.',
      'A 14 to 64 tooth gear gives the motor its margin.',
    ],
    explodeKicker: 'Exploded view',
    explodeTitle: 'Every part, where it goes.',
    explodeAlt: 'The watch winder’s parts sliding apart as the page scrolls',
    parts: [
      ['Drum and cushion', 'PLA · TPU'],
      ['608 bearings', '22 × 8 × 7 mm'],
      ['Pinion, 14 teeth', 'module 1.5'],
      ['Back plate and base', 'no supports'],
    ],
    editKicker: 'Live, with the agent',
    editTitle: '“Make the dome 8 mm taller.”',
    editFile: 'nightlight.c2f',
    editLineBefore: '// Height of the dome above the top.',
    editLineAfter: '// Thickness of the dome’s shell.',
    reportStart: '▸ studio_state',
    reportWorking: '… edit_code · rendering',
    reportDone: '✓ edit_code · line 29 · rendered, no errors',
    heightLabel: 'height',
    domeAlt: 'The night light, its dome growing from 26 to 34 mm',
    numbersTitle: 'What the plan changed',
    margin: 'torque margin, from 0.77 with the motor alone',
    load: 'left on the motor’s bushing, from 17.5 N',
    tip: 'it can lean before it tips over',
  },
  checked: {
    title: 'Checked before the first layer',
    text:
      'Before anything is drawn, the plan thinks the object through as an engineer would: masses, balance, forces, torque. For the watch winder it found the motor short of torque and carrying the watch, and fixed both. Then the design is measured against the brief.',
    how: 'Through the MCP server, in Claude Code: the plan_object prompt.',
    cardTitle: 'check_brief · watch_winder.c2f',
    rows: [
      ['holds', 'mass < 600 g', '476.6 g'],
      ['holds', 'tipping > 25°', '37.0°'],
      ['holds', 'torque margin ≥ 2', '9.41'],
      ['check', 'quiet enough for a bedroom', 'you'],
    ],
    sheetAlt:
      'The watch winder’s sheet of views at one scale: diagonal, front, right, top and a section, 112 × 154.33 × 161.8 mm',
  },
  review: {
    title: 'You keep the last word',
    text:
      'Every change the agent makes is one undo step, and one button undoes all of an answer. Ask it to wait for you, and each change shows as a diff, the new model over the old one, before you accept it.',
    alt: 'The agent’s change under review: line 50 of the code, removed and added, with Decline and Accept',
  },
  examples: {
    title: 'Print one tonight',
    all: 'All examples on GitHub',
    files: '.c2f → STL · 3MF',
    plateFiles: 'for_print = true',
    items: {
      nightlight: {
        name: 'Night light',
        alt: 'The night light: a round base and a white dome, seen from the diagonal',
      },
      winder: {
        name: 'Watch winder',
        alt: 'The watch winder’s frame with its stepper motor and boards inside, seen through',
      },
      plate: {
        name: 'On the plate',
        alt: 'The night light’s base, top and dome laid out to print',
      },
    },
  },
  cta: {
    title: 'Free and open source',
    text: 'MIT or Apache-2.0 · every Mac, Apple silicon and Intel · Windows and Linux from source',
    button: 'Download',
    yourTurn: 'Your turn.',
  },
  footer: {
    ready: 'Ready',
    ways: 'made in Rust, renders as you type',
    releases: 'Releases',
    licence: 'Licence',
  },
  download: {
    title: 'Download Code2Form',
    lead: 'One app for every Mac, Apple silicon and Intel, on macOS 11 or later. Free and open source.',
    released: 'Released',
    latest: 'Latest release',
    offline:
      'The release could not be looked up while this page was built. All the downloads are on the releases page.',
    allReleases: 'All releases on GitHub',
    files: 'Files',
    app: 'The app',
    appNote: 'Code2Form.app: the editor and its 3D view.',
    cli: 'The command line',
    cliNote: 'c2f: renders and exports from a terminal, and serves the MCP tools to an agent.',
    sums: 'Checksums',
    sumsNote: 'to check a download with shasum -a 256',
    sha: 'SHA-256',
    firstTitle: 'The first launch on a Mac',
    firstLead:
      'Code2Form is not notarized by Apple yet, so macOS stops it the first time. Once you allow it, it opens normally from then on.',
    firstSteps: [
      'Unzip Code2Form and move Code2Form.app into Applications.',
      'Open it. macOS says it cannot check it: click Done.',
      'Open System Settings › Privacy & Security, scroll down, and click Open Anyway next to Code2Form. Confirm with your password.',
    ],
    firstTerminal: 'Or, in a terminal, take the app out of quarantine:',
    mcpTitle: 'The MCP server, for Claude Code',
    mcpLead:
      'c2f mcp lets an agent such as Claude Code write a model, render it, look at a picture of it, check it and export it, without opening a window.',
    mcpSteps: [
      'Unzip c2f, take it out of quarantine, and put it in a folder on your PATH:',
      'Add it to Claude Code:',
    ],
    mcpMore: 'The tools and prompts it offers',
    sourceTitle: 'Windows and Linux: build from source',
    sourceLead:
      'There are no Windows or Linux downloads yet, but the app and c2f build there with Rust 1.85 or later.',
    sourceNote: 'The app is then target/release/code2form, and the command line target/release/c2f.',
    sourceMore: 'Building, in the README',
  },
};

export type Strings = typeof en;
export default en;
