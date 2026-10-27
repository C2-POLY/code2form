import type { Strings } from './en';

// Every string the French pages show, with the same keys as en.ts.
const fr: Strings = {
  meta: {
    title: 'Code2Form : la conception 3D paramétrique en code, pour votre imprimante 3D',
    description:
      'Libre et gratuit. Décrivez une pièce en quelques lignes, voyez-la se calculer pendant que vous tapez, vérifiez-la et exportez-la en STL ou 3MF pour votre trancheur. Ouvre vos fichiers .scad tels quels.',
    downloadTitle: 'Télécharger Code2Form',
    downloadDescription:
      'Téléchargez Code2Form pour macOS (Apple silicon et Intel), installez son serveur MCP pour Claude Code, ou compilez-le depuis les sources sous Windows et Linux.',
  },
  a11y: {
    skip: 'Aller au contenu',
    mainNav: 'Principale',
    theme: 'Passer du thème sombre au thème clair',
    language: 'Langue',
    progress: 'Votre avancée dans la page',
  },
  nav: {
    overview: 'Aperçu',
    examples: 'Exemples',
    docs: 'Docs',
    download: 'Télécharger',
    github: 'GitHub',
  },
  hero: {
    badge: 'libre et gratuit',
    titleStart: 'Décrivez une pièce en quelques lignes de code.',
    titleAccent: 'Imprimez-la ce soir.',
    lead:
      'Code2Form calcule le modèle pendant que vous tapez, pèse la pièce, vérifie ses engrenages et son câblage avant la première couche, puis l’exporte en STL et 3MF pour votre trancheur. Vos fichiers .scad s’ouvrent tels quels.',
    downloadMac: 'Télécharger pour macOS',
    printExample: 'Imprimer un exemple',
    shotAlt:
      'La fenêtre de Code2Form : une veilleuse dans la vue 3D, et la discussion avec l’agent qui flotte par-dessus, avec la ligne de code qu’il a modifiée',
    shotCaption: 'La discussion avec l’agent, qui flotte sur la vue.',
  },
  ways: {
    title: 'Un modèle, trois façons de le faire',
    code: {
      label: 'Code',
      title: 'En code',
      text:
        'De courts programmes dans le langage .scad. Les nombres annotés d’un commentaire deviennent des curseurs, et la vue suit chaque frappe.',
    },
    mouse: {
      title: 'À la souris',
      tools: ['Formes', 'Déplacer', 'Esquisse', 'Extruder'],
      text:
        'Ajoutez des formes depuis une barre d’outils, déplacez-les par leurs poignées, dessinez une esquisse et extrudez-la dans la vue. Chaque geste s’écrit dans le code, une étape d’annulation à la fois.',
    },
    agent: {
      ask: 'Rends le dôme 8 mm plus haut.',
      title: 'En demandant',
      text:
        'Une discussion flotte sur la vue et modifie le modèle ouvert en direct, avec votre abonnement Claude via Claude Code, ou une clé d’API.',
    },
  },
  story: {
    intro: 'Le remontoir de montre, conçu dans Code2Form',
    titleStart: 'Réfléchi.',
    titleAccent: 'Puis imprimé.',
    lead: 'Faites défiler pour le tourner, le démonter, et voir ce que le plan a changé.',
    turnAlt: 'Le remontoir de montre : un tambour incliné sur un bâti imprimé, qui tourne au fil du défilement',
    captions: [
      'Un remontoir de montre pour une table de chevet.',
      'Le tambour tourne sur trois roulements 608, pas sur le moteur.',
      'Un engrenage de 14 et 64 dents donne au moteur sa marge.',
    ],
    explodeKicker: 'Vue éclatée',
    explodeTitle: 'Chaque pièce, à sa place.',
    explodeAlt: 'Les pièces du remontoir qui s’écartent au fil du défilement',
    parts: [
      ['Tambour et coussin', 'PLA · TPU'],
      ['Roulements 608', '22 × 8 × 7 mm'],
      ['Pignon, 14 dents', 'module 1,5'],
      ['Plaque et socle', 'sans supports'],
    ],
    editKicker: 'En direct, avec l’agent',
    editTitle: '« Rends le dôme 8 mm plus haut. »',
    editFile: 'nightlight.c2f',
    editLineBefore: '// Height of the dome above the top.',
    editLineAfter: '// Thickness of the dome’s shell.',
    reportStart: '▸ studio_state',
    reportWorking: '… edit_code · rendu en cours',
    reportDone: '✓ edit_code · ligne 29 · rendu sans erreur',
    heightLabel: 'hauteur',
    domeAlt: 'La veilleuse, dont le dôme passe de 26 à 34 mm',
    numbersTitle: 'Ce que le plan a changé',
    margin: 'de marge de couple, contre 0,77 avec le moteur seul',
    load: 'sur le palier du moteur, contre 17,5 N',
    tip: 'd’inclinaison avant qu’il ne bascule',
  },
  checked: {
    title: 'Vérifié avant la première couche',
    text:
      'Avant de dessiner quoi que ce soit, le plan pense l’objet comme le ferait un ingénieur : masses, équilibre, efforts, couple. Pour le remontoir, il a trouvé un moteur trop juste en couple et qui portait la montre, et a corrigé les deux. Le modèle est ensuite mesuré face au cahier des charges.',
    how: 'Par le serveur MCP, dans Claude Code : le prompt plan_object.',
    cardTitle: 'check_brief · watch_winder.c2f',
    rows: [
      ['tient', 'masse < 600 g', '476,6 g'],
      ['tient', 'basculement > 25°', '37,0°'],
      ['tient', 'marge de couple ≥ 2', '9,41'],
      ['à voir', 'assez silencieux pour une chambre', 'vous'],
    ],
    sheetAlt:
      'La planche de vues du remontoir à une même échelle : diagonale, face, droite, dessus et une coupe, 112 × 154,33 × 161,8 mm',
  },
  review: {
    title: 'Vous gardez le dernier mot',
    text:
      'Chaque modification de l’agent est une étape d’annulation, et un bouton annule toute une réponse. Demandez-lui de vous attendre, et chaque modification s’affiche en diff, le nouveau modèle sur l’ancien, avant que vous l’acceptiez.',
    alt: 'La modification de l’agent en revue : la ligne 50 du code, retirée et ajoutée, avec Decline et Accept',
  },
  examples: {
    title: 'À imprimer ce soir',
    all: 'Tous les exemples sur GitHub',
    files: '.c2f → STL · 3MF',
    plateFiles: 'for_print = true',
    items: {
      nightlight: {
        name: 'Veilleuse',
        alt: 'La veilleuse : un socle rond et un dôme blanc, vus en diagonale',
      },
      winder: {
        name: 'Remontoir de montre',
        alt: 'Le bâti du remontoir avec son moteur pas à pas et ses cartes, vus par transparence',
      },
      plate: {
        name: 'Sur le plateau',
        alt: 'Le socle, le dessus et le dôme de la veilleuse disposés pour l’impression',
      },
    },
  },
  cta: {
    title: 'Libre et gratuit',
    text: 'MIT ou Apache-2.0 · tous les Mac, Apple silicon et Intel · Windows et Linux depuis les sources',
    button: 'Télécharger',
    yourTurn: 'À vous.',
  },
  footer: {
    ready: 'Prêt',
    ways: 'écrit en Rust, calcule pendant que vous tapez',
    releases: 'Versions',
    licence: 'Licence',
  },
  download: {
    title: 'Télécharger Code2Form',
    lead: 'Une seule app pour tous les Mac, Apple silicon et Intel, sous macOS 11 ou plus récent. Libre et gratuite.',
    released: 'Publiée le',
    latest: 'Dernière version',
    offline:
      'La version n’a pas pu être consultée lors de la construction de cette page. Tous les téléchargements sont sur la page des versions.',
    allReleases: 'Toutes les versions sur GitHub',
    files: 'Fichiers',
    app: 'L’app',
    appNote: 'Code2Form.app : l’éditeur et sa vue 3D.',
    cli: 'La ligne de commande',
    cliNote: 'c2f : calcule et exporte depuis un terminal, et sert les outils MCP à un agent.',
    sums: 'Sommes de contrôle',
    sumsNote: 'pour vérifier un téléchargement avec shasum -a 256',
    sha: 'SHA-256',
    firstTitle: 'Le premier lancement sur Mac',
    firstLead:
      'Code2Form n’est pas encore notarié par Apple : macOS l’arrête la première fois. Une fois autorisé, il s’ouvre normalement.',
    firstSteps: [
      'Décompressez Code2Form et placez Code2Form.app dans Applications.',
      'Ouvrez-le. macOS indique qu’il ne peut pas le vérifier : cliquez sur Terminé.',
      'Ouvrez Réglages Système › Confidentialité et sécurité, descendez, et cliquez sur Ouvrir quand même à côté de Code2Form. Confirmez avec votre mot de passe.',
    ],
    firstTerminal: 'Ou, dans un terminal, sortez l’app de la quarantaine :',
    mcpTitle: 'Le serveur MCP, pour Claude Code',
    mcpLead:
      'c2f mcp permet à un agent comme Claude Code d’écrire un modèle, de le calculer, d’en regarder une image, de le vérifier et de l’exporter, sans ouvrir de fenêtre.',
    mcpSteps: [
      'Décompressez c2f, sortez-le de la quarantaine, et placez-le dans un dossier de votre PATH :',
      'Ajoutez-le à Claude Code :',
    ],
    mcpMore: 'Les outils et prompts qu’il propose',
    sourceTitle: 'Windows et Linux : compiler depuis les sources',
    sourceLead:
      'Il n’y a pas encore de téléchargement pour Windows ni Linux, mais l’app et c2f s’y compilent avec Rust 1.85 ou plus récent.',
    sourceNote: 'L’app est alors target/release/code2form, et la ligne de commande target/release/c2f.',
    sourceMore: 'La compilation, dans le README',
  },
};

export default fr;
