// Marquee's patches to a Sonarr or Radarr checkout, before its web app is built (run by ui/build.sh):
//   1. the Encore theme (ui/arr/encore.js) is every theme the app can pick, so the pages always use the kit's look;
//   2. the kit's font in place of Roboto;
//   3. index.html loads the kit's colour tokens (Content/encore-tokens.css, copied in by ui/build.sh).
// Each step checks that the upstream text it changes is still there, so a newer upstream fails the build loudly
// instead of quietly losing the look.
import { copyFileSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const [src] = process.argv.slice(2);
if (!src) {
  console.error('usage: node patch.mjs <sonarr or radarr checkout>');
  process.exit(2);
}
const here = path.dirname(new URL(import.meta.url).pathname);
const front = path.join(src, 'frontend', 'src');

/** Replaces `from` with `to` in a file; stops the build when `from` is missing. */
function replace(file, from, to) {
  const full = path.join(front, file);
  const text = readFileSync(full, 'utf8');
  if (!text.includes(from)) {
    console.error(`ui/arr/patch.mjs: "${from}" is no longer in ${file}; update the patch for this upstream.`);
    process.exit(1);
  }
  writeFileSync(full, text.replace(from, to));
}

copyFileSync(path.join(here, 'encore.js'), path.join(front, 'Styles', 'Themes', 'encore.js'));
writeFileSync(
  path.join(front, 'Styles', 'Themes', 'index.js'),
  "// Marquee: every theme is Encore (ui/arr/encore.js), the kit's look.\n" +
    "import * as encore from './encore';\n\n" +
    'export default {\n  auto: encore,\n  light: encore,\n  dark: encore\n};\n'
);

replace(
  path.join('Styles', 'Variables', 'fonts.js'),
  `defaultFontFamily: 'Roboto, "open sans", "Helvetica Neue", Helvetica, Arial, sans-serif'`,
  `defaultFontFamily: 'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif'`
);

replace(
  'index.ejs',
  '<link rel="stylesheet" type="text/css" href="/Content/Fonts/fonts.css">',
  '<link rel="stylesheet" type="text/css" href="/Content/Fonts/fonts.css">\n' +
    '    <link rel="stylesheet" type="text/css" href="/Content/encore-tokens.css">\n' +
    '    <style>html { background-color: var(--bg); color-scheme: dark; }</style>'
);
console.log(`Patched ${src} with the Encore look.`);
