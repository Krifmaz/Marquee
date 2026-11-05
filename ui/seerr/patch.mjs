// Marquee's patches to a Seerr checkout, before its web app is built (run by ui/build.sh):
//   1. a static build (`output: 'export'`) served by Marquee at /requests, so the image needs no Node server;
//   2. the sign-in and public settings are read in the browser (they were read on Seerr's own server), and a
//      visitor who is not signed in goes to Marquee's sign-in at /;
//   3. the movie, TV and collection pages read their data in the browser (they already do through SWR);
//   4. the app's own files (/images, /logo_full.svg, /sw.js, …) are asked for under /requests;
//   5. every change Seerr's web app sends carries Marquee's CSRF header (X-Requested-With: Marquee);
//   6. the Encore look: Tailwind's grays and indigos come from the kit's colour tokens, and the kit's font.
// Each step checks that the upstream text it changes is still there, so a newer upstream fails the build loudly.
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const [src, tokensFile] = process.argv.slice(2);
if (!src || !tokensFile) {
  console.error('usage: node patch.mjs <seerr checkout> <kit tokens.css>');
  process.exit(2);
}
/** Where Marquee serves the Seerr web app. */
const BASE = '/requests';

/** Reads a file of the checkout. */
const read = (file) => readFileSync(path.join(src, file), 'utf8');
/** Writes a file of the checkout. */
const write = (file, text) => writeFileSync(path.join(src, file), text);

/** Replaces `from` (a string or a regular expression) in a file; stops the build when it is missing. */
function replace(file, from, to) {
  const text = read(file);
  const found = typeof from === 'string' ? text.includes(from) : from.test(text);
  if (!found) {
    console.error(`ui/seerr/patch.mjs: ${from} is no longer in ${file}; update the patch for this upstream.`);
    process.exit(1);
  }
  write(file, text.replace(from, to));
}

// 1. a static build at /requests
replace(
  'next.config.ts',
  'const nextConfig: NextConfig = {',
  `const nextConfig = {\n  output: 'export',\n  basePath: '${BASE}',\n  typescript: { ignoreBuildErrors: true },`
);
replace('next.config.ts', 'images: {', 'images: {\n    unoptimized: true,');

// 2. sign-in and settings in the browser
replace('src/pages/_app.tsx', /CoreApp\.getInitialProps = async[\s\S]*?\nexport default CoreApp;\n/, `
/** Marquee: what Seerr's server used to hand the app, read in the browser before the first page is drawn. */
interface MarqueeBoot {
  user?: User;
  messages: MessagesType;
  locale: AvailableLocale;
  currentSettings: PublicSettingsResponse;
}

/** Marquee: the app, drawn once the account and the public settings are in; not signed in → Marquee's sign-in. */
const MarqueeApp = (props: AppProps) => {
  const [boot, setBoot] = useState<MarqueeBoot | null>(null);
  useEffect(() => {
    (async () => {
      const settings = await axios.get<PublicSettingsResponse>('/api/v1/settings/public');
      let user: User | undefined;
      try {
        user = (await axios.get<User>('/api/v1/auth/me')).data;
      } catch {
        window.location.href = '/?next=' + encodeURIComponent(window.location.pathname + window.location.search);
        return;
      }
      const locale = (user?.settings?.locale || settings.data.locale) as AvailableLocale;
      const messages = await loadLocaleData(locale);
      await polyfillIntl();
      setBoot({ user, messages, locale, currentSettings: settings.data });
    })();
  }, []);
  if (!boot) {
    return null;
  }
  const Core = CoreApp as unknown as React.ComponentType<ExtendedAppProps>;
  return <Core {...(props as AppProps)} {...(boot as MarqueeBoot & { user: User })} />;
};

export default MarqueeApp;
`);

// 3. the detail pages read their data in the browser
for (const page of ['movie/[movieId]', 'tv/[tvId]', 'collection/[collectionId]']) {
  replace(`src/pages/${page}/index.tsx`, /export const getServerSideProps[\s\S]*?\n};\n/, '');
}

// 4. the app's own files under /requests (the API stays at /api/v1)
const publicFiles = readdirSync(path.join(src, 'public')).map((name) => name.replace(/[.]/g, '\\.'));
const ownFile = new RegExp(`(['"\`])/(${publicFiles.join('|')})(?=[/'"\`?#])`, 'g');
/** Every .ts and .tsx file under a folder. */
function sources(dir) {
  return readdirSync(dir).flatMap((name) => {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return /\.tsx?$/.test(name) ? [full] : [];
  });
}
let moved = 0;
for (const file of sources(path.join(src, 'src'))) {
  const text = readFileSync(file, 'utf8');
  const next = text.replace(ownFile, (_, quote, name) => {
    moved += 1;
    return `${quote}${BASE}/${name}`;
  });
  if (next !== text) writeFileSync(file, next);
}
if (moved === 0) {
  console.error('ui/seerr/patch.mjs: no paths to the app\'s own files were found; update the patch.');
  process.exit(1);
}

// 5. Marquee's CSRF header on every request
replace(
  'src/pages/_app.tsx',
  "import { SWRConfig } from 'swr';",
  "import { SWRConfig } from 'swr';\n\naxios.defaults.headers.common['X-Requested-With'] = 'Marquee';"
);

// 5b. "Marquee updated": compare the server's version with the one it had when the page opened (Seerr compared
//     it with the version the web app was built from, which a static build at a pinned commit never matches)
const checker = 'src/components/StatusChecker/index.tsx';
if (read(checker).split('data.commitTag !== process.env.commitTag').length !== 3) {
  console.error(`ui/seerr/patch.mjs: ${checker} no longer compares commitTag twice; update the patch.`);
  process.exit(1);
}
write(
  checker,
  read(checker).replaceAll(
    'data.commitTag !== process.env.commitTag',
    'data.commitTag !== firstCommitTag(data.commitTag)'
  )
);
replace(
  checker,
  "import useSWR from 'swr';",
  "import useSWR from 'swr';\n\n" +
    "/** Marquee: the server's version when the page opened. */\nconst seenCommitTag: { tag?: string } = {};\n" +
    '/** Marquee: the first version this page saw. */\n' +
    'const firstCommitTag = (tag: string): string => {\n' +
    '  if (seenCommitTag.tag === undefined) seenCommitTag.tag = tag;\n' +
    '  return seenCommitTag.tag;\n' +
    '};'
);

// 6. the Encore look
/** The kit's default theme (the first `:root` block of tokens.css): name → value. */
const tokens = Object.fromEntries(
  [...readFileSync(tokensFile, 'utf8').split(/\n}\n/)[0].matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)].map((m) => [
    m[1],
    m[2].trim(),
  ])
);
/** A token's colour as `[r, g, b]`. */
function rgb(name) {
  const hex = tokens[name];
  if (!/^#[0-9a-f]{6}$/i.test(hex ?? '')) {
    console.error(`ui/seerr/patch.mjs: the kit token --${name} is not a #rrggbb colour (${hex}).`);
    process.exit(1);
  }
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
}
/** `a` mixed with `b`: `t` of `a`, the rest `b`, as Tailwind's `r g b` channels. */
function mix(a, b, t) {
  const x = rgb(a);
  const y = rgb(b);
  return x.map((v, i) => Math.round(v * t + y[i] * (1 - t))).join(' ');
}
const shades = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950];
/** Grays: the kit's surfaces and text. Indigos and purples (Seerr's accents): the kit's second accent, darkened
 *  toward the background for the filled buttons so white text stays readable. */
const c = (name) => rgb(name).join(' ');
const gray = {
  50: c('text'), 100: c('text'), 200: c('text'), 300: c('text'), 400: c('muted'), 500: c('faint'),
  600: c('border'), 700: c('panel-2'), 800: c('panel'), 900: c('bg'), 950: c('bg'),
};
const indigo = {
  50: c('accent'), 100: c('accent'), 200: mix('accent', 'accent-2', 0.5), 300: mix('accent-2', 'accent', 0.7),
  400: c('accent-2'), 500: mix('accent-2', 'bg', 0.85), 600: mix('accent-2', 'bg', 0.5),
  700: mix('accent-2', 'bg', 0.4), 800: mix('accent-2', 'bg', 0.3), 900: mix('accent-2', 'bg', 0.2),
  950: mix('accent-2', 'bg', 0.12),
};
const vars = [
  ...shades.map((s) => `  --mq-gray-${s}: ${gray[s]};`),
  ...shades.map((s) => `  --mq-indigo-${s}: ${indigo[s]};`),
].join('\n');
replace(
  'src/styles/globals.css',
  '@tailwind base;',
  `/* Marquee: the kit's Encore colours as Tailwind channels (ui/seerr/patch.mjs) */\n` +
    `:root {\n${vars}\n}\n\n@tailwind base;`
);
// globals.css's own colours (scrollbars, the sidebar's blue-gray gradient) to the same tokens
const ownColours = [
  ['background: linear-gradient(180deg, rgba(31, 41, 55, 1) 0%, #131928 100%);',
    'background: linear-gradient(180deg, rgb(var(--mq-gray-800)) 0%, rgb(var(--mq-gray-900)) 100%);'],
  ['#1f2937', 'rgb(var(--mq-gray-800))'],
  ['#4b5563', 'rgb(var(--mq-gray-600))'],
  ['#6b7280', 'rgb(var(--mq-gray-500))'],
];
for (const [from, to] of ownColours) {
  if (!read('src/styles/globals.css').includes(from)) {
    console.error(`ui/seerr/patch.mjs: ${from} is no longer in src/styles/globals.css; update the patch.`);
    process.exit(1);
  }
  write('src/styles/globals.css', read('src/styles/globals.css').replaceAll(from, to));
}
const palette = (name) =>
  `{ ${shades.map((s) => `${s}: 'rgb(var(--mq-${name}-${s}) / <alpha-value>)'`).join(', ')} }`;
replace(
  'tailwind.config.js',
  'extend: {',
  `extend: {\n      colors: { gray: ${palette('gray')}, indigo: ${palette('indigo')}, purple: ${palette('indigo')} },`
);
replace(
  'tailwind.config.js',
  "sans: ['Inter Variable', ...defaultTheme.fontFamily.sans],",
  "sans: ['system-ui', '-apple-system', 'Segoe UI', 'Roboto', 'Ubuntu', 'sans-serif'],"
);
console.log(`Patched ${src}: static build at ${BASE}, ${moved} file paths moved, Encore colours.`);
