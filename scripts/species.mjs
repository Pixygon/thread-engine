#!/usr/bin/env node
// Species from the Codex — prose in, a grow recipe out, the turntable decides.
//
//   node scripts/species.mjs <codex-slug | prose.md | "prose…"> [--out recipes/<slug>.grow.json]
//                            [--seed n] [--rounds 2] [--model claude-opus-5] [--no-judge]
//
// Roadmap step 5 for Grove. The loop is the one every other proof in this repo
// runs by hand: an agent drafts the rules, `thread grow` grows them and renders
// the turntable, the life sheet and the year sheet, and a second look at those
// sheets — against the prose — says accept or revise, with the revision as a
// patch to the recipe. Two rounds is usually enough; the founder is the final
// judge, and the sheets are left where they can be looked at.
//
// Drafting goes through the local `claude` CLI on the founder's subscription,
// the way `pearl ship --local-draft` does — never a direct API call (the
// credit wall killed every ship that tried, 2026-09-12). The drafter gets the
// species contract, the three recipes in `crates/grove/recipes` as worked
// examples, and the prose; it may use nothing else. The judge gets the prose,
// the recipe and the sheets, and may Read the images and nothing else.
//
// Nothing here invents lore: the prose is the Codex's, or the file you hand
// it. If a plant is not in the Codex yet, write its entry first
// (`pearl codex push`), then draft from the slug.
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const RECIPES = path.join(ROOT, 'crates', 'grove', 'recipes');

const args = process.argv.slice(2);
const flag = (name, dflt) => {
    const i = args.indexOf(name);
    if (i < 0) return dflt;
    const v = args[i + 1];
    args.splice(i, 2);
    return v ?? dflt;
};
const has = (name) => {
    const i = args.indexOf(name);
    if (i < 0) return false;
    args.splice(i, 1);
    return true;
};
const model = flag('--model', process.env.PEARL_LOCAL_DRAFT_MODEL || 'claude-opus-5');
const rounds = Math.max(1, parseInt(flag('--rounds', '2'), 10) || 2);
const seedFlag = flag('--seed', null);
const outFlag = flag('--out', null);
const noJudge = has('--no-judge');
const source = args[0];
if (!source) {
    console.error('usage: node scripts/species.mjs <codex-slug | prose.md | "prose…"> [--out recipes/x.grow.json] [--seed n] [--rounds 2] [--model m] [--no-judge]');
    process.exit(2);
}

// ── The prose ───────────────────────────────────────────────────────────────
function prose(src) {
    if (existsSync(src)) {
        return { slug: path.basename(src).replace(/\.(md|txt)$/, ''), text: readFileSync(src, 'utf8'), from: `file ${src}` };
    }
    if (/^[a-z0-9][a-z0-9-]*$/.test(src)) {
        const r = spawnSync('pearl', ['codex', 'get', src], { encoding: 'utf8' });
        if (r.status === 0 && r.stdout.trim().startsWith('{')) {
            const e = JSON.parse(r.stdout);
            const body = e.description || e.body || '';
            const text = `# ${e.name || src}\n${e.summary ? `\n${e.summary}\n` : ''}\n${body}`;
            return { slug: e.slug || src, text, from: `codex ${e.slug || src}`, entry: e };
        }
        console.error(`✗ no Codex entry '${src}' (and no file by that name). Write the entry first: pearl codex push <bundle.md>`);
        process.exit(1);
    }
    return { slug: slugify(src.split(/\s+/).slice(0, 3).join(' ')), text: src, from: 'the command line' };
}
const slugify = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'plant';

// ── The species contract, as the drafter sees it ────────────────────────────
const CONTRACT = `You write a GROW RECIPE for the Thread's Grove: a JSON object that is a plant SPECIES plus one individual. A plant is rules, not vertices — from one recipe and one seed the same plant grows everywhere. Your job is to turn the prose into the rules that would grow THAT plant, faithfully, and nothing the prose does not support.

Reply with ONLY one JSON object, no prose, no fences. Every field is optional (defaults are sensible for a broadleaf tree); include what the prose earns. Fields:

name (kebab-case string)            seed (u32: which individual; any number)
height (m, the TRUNK up to its first fork — a tree's height is this plus its limbs; 1.5–3 is a tree, 0.2–0.6 a shrub)
trunk_radius (m at the ground; 0.15–0.6 for a tree)
levels (generations of branching beyond the trunk, 2–5)
forks ([per level] children starting AT the parent's tip that carry it on; a real tree is mostly forks: [3,2,2,2])
branches ([per level] laterals sprouting along the parent between sprout_from and its tip: [1,1,1,0])
angle ([per level] degrees from the parent, 25–45)
length ([per level] child length as a fraction of the parent, 0.5–1.0)
curve ([per level] degrees of arc along a branch — gnarl is high curve: 10–40)
trunk_curve (degrees), lean (degrees from vertical at the ground)
taper (tip radius / base radius, 0.5–0.85), child_radius (0.4–0.7)
gravity (droop per metre; negative LIFTS, e.g. -0.16 for a crystal tree whose tips reach up)
wobble (0–0.4, wander on top of the arc, spread over the whole branch)
sprout_from (0–1, how far up the parent the first lateral sprouts)
flare (root flare, 0.5–1.3), sway (0–1, how much the tips move in wind)
sides, segments (mesh detail; leave at defaults 10, 6 unless the prose is about a huge tree)
color ([r,g,b,a] flat wood colour when there is no bark), emissive (0–1 glow)
bark: a texture recipe { "kind": "fbm|voronoi|bricks|wood|veins|checker|flat", "scale": 4–10, "octaves": 3–6, "seed": n, "colors": [[r,g,b],…] (a ramp of 3–5 stops, dark→light), "roughness": [lo,hi], "metallic": [lo,hi], "height": 0.3–1.0 }
leaves: null for a bare/crystal/dead species, else { "per_tip": 4–10, "along": 0–0.6, "along_count": 3–8, "depth": 1|2, "length": m, "width": m, "fold": m, "spread": deg, "droop": 0–1, "jitter": 0–0.4, "color": [r,g,b,a], "color_tip": [r,g,b,a], "color_autumn": [r,g,b,a], "sway": 0–0.5 }
evergreen (bool; true keeps the crown all year)
seasons_to_grown (seasons to full size, 8–30), sprout_size (0.03–0.1), seasons_of_life (whole arc, 60–400)
blooms / fruit: null, or { "per_tip": 1–3, "share": 0–1 (what share of tips bear), "depth": 1|2, "window": [start,end] in the year 0..1 (bud 0–0.1, leaf 0.1–0.25, bloom 0.25–0.4, fruit 0.4–0.7, seed drop 0.7–0.85, bare 0.85–1; a window may wrap), "along": 0–0.4 }. Fruit and blooms are SOCKETS — what hangs there is a separate model — so declare them wherever the prose says the plant bears something (lanterns, pods, crystals, berries).
wither: { "gnarl": deg, "sag": droop, "darken": 0–1, "drain": 0–1 } — what this species looks like dead standing (a prose that describes a withered/dead form goes here, NOT in a second species).

Rules that matter: 'height' is the trunk to its first fork; 'forks' split the trunk into limbs; 'curve' bends each branch in an arc; 'branches' adds laterals; 'gravity' near zero lets 'curve' do the gnarl; a big droop plus a lean tips a crown over. Colours are linear 0..1. Keep every number inside the ranges above unless the prose clearly demands otherwise. Do not add fields that are not listed.`;

// The worked examples: every hand-made recipe here — except the one for the
// plant being drafted, and any that is the same species under another name
// (the withered tree is the lantern tree with one flag), or the draft would
// be a copy and the proof would prove nothing.
function examples(slug) {
    const all = ['oak', 'lantern-tree', 'withered-tree']
        .map((n) => ({ n, p: path.join(RECIPES, `${n}.grow.json`) }))
        .filter(({ p }) => existsSync(p))
        .map(({ n, p }) => ({ n, text: readFileSync(p, 'utf8').trim(), json: JSON.parse(readFileSync(p, 'utf8')) }));
    const species = (j) => {
        const s = { ...j };
        for (const k of ['name', 'seed', 'age', 'season', 'withered', 'cut', 'taken']) delete s[k];
        return JSON.stringify(s);
    };
    const same = (a, b) => a.n === slug || a.json.name === slug || (b && species(a.json) === species(b.json));
    const excluded = all.filter((e) => same(e, null));
    const kept = all.filter((e) => !excluded.some((x) => same(e, x)));
    if (excluded.length) console.log(`      (leaving ${all.length - kept.length} example(s) out: the same species)`);
    return kept.map((e) => `### ${e.n}.grow.json\n${e.text}`).join('\n\n');
}

// ── The local Claude ────────────────────────────────────────────────────────
function claude(user, system, allowedTools) {
    const out = execFileSync('claude', ['-p', user, '--model', model, '--output-format', 'json', '--append-system-prompt', system, '--allowedTools', allowedTools], {
        encoding: 'utf8',
        maxBuffer: 64 * 1024 * 1024,
        timeout: 300_000,
        stdio: ['ignore', 'pipe', 'pipe'],
        cwd: ROOT,
    });
    const wrapper = JSON.parse(out);
    const text = String(wrapper.result || '');
    const m = text.match(/\{[\s\S]*\}/);
    if (!m) throw new Error(`no JSON in the reply: ${text.slice(0, 200)}`);
    return { json: JSON.parse(m[0]), cost: wrapper.total_cost_usd || 0 };
}

// ── The grower ──────────────────────────────────────────────────────────────
function threadBin() {
    // Whichever build is newer: a stale release binary is worse than none.
    const built = ['release', 'debug']
        .map((p) => path.join(ROOT, 'target', p, 'thread'))
        .filter((b) => existsSync(b))
        .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
    if (built.length) return built[0];
    console.log('  building thread-cli…');
    const r = spawnSync('cargo', ['build', '-p', 'thread-cli'], { cwd: ROOT, stdio: 'inherit' });
    if (r.status !== 0) throw new Error('cargo build failed');
    return path.join(ROOT, 'target', 'debug', 'thread');
}

// A crop is sockets, not geometry, so on a bare turntable a fruiting species
// looks like it bears nothing. For the proof a plain sphere is hung at every
// fruit socket — a stand-in, not the fruit — so the judge can see where and
// how many. Carved once, kept beside the drafts.
function placeholderFruit() {
    const p = path.join(ROOT, 'target', 'species', '_fruit.glb');
    if (existsSync(p)) return p;
    mkdirSync(path.dirname(p), { recursive: true });
    const r = spawnSync(threadBin(), ['model', '--lib', 'sphere', '-a', '[0.16]', '-o', p], { cwd: ROOT, encoding: 'utf8' });
    return r.status === 0 && existsSync(p) ? p : null;
}

function grow(recipePath, outDir) {
    const stem = path.join(outDir, 'plant');
    const argv = ['grow', recipePath, '-o', `${stem}.glb`, '--preview', `${stem}.png`, '--life', `${stem}.life.png`, '--year', `${stem}.year.png`, '--views', '3'];
    let recipe = {};
    try { recipe = JSON.parse(readFileSync(recipePath, 'utf8')); } catch {}
    if (recipe.fruit || recipe.blooms) {
        const fruit = placeholderFruit();
        if (fruit) argv.push('--hang', fruit, '--hang-kind', recipe.fruit ? 'fruit' : 'bloom', '--hang-count', '0', '--hang-glow', '1.2');
    }
    const r = spawnSync(threadBin(), argv, {
        cwd: ROOT,
        encoding: 'utf8',
    });
    const log = (r.stdout || '') + (r.stderr || '');
    if (r.status !== 0) return { ok: false, log };
    return { ok: true, log, sheets: [`${stem}.png`, `${stem}.life.png`, `${stem}.year.png`] };
}

// ── The loop ────────────────────────────────────────────────────────────────
const src = prose(source);
const slug = slugify(src.slug);
const outPath = outFlag ? path.resolve(outFlag) : path.join(RECIPES, `${slug}.grow.json`);
const workDir = path.join(ROOT, 'target', 'species', slug);
mkdirSync(workDir, { recursive: true });
console.log(`\n🌱 ${slug} — from ${src.from}\n`);

let recipe = null;
let feedback = '';
let cost = 0;
for (let round = 1; round <= rounds; round++) {
    // Draft, or revise from the judge's notes.
    const ask = recipe
        ? `# PROSE\n${src.text}\n\n# CURRENT RECIPE\n${JSON.stringify(recipe, null, 1)}\n\n# THE TURNTABLE'S VERDICT\n${feedback}\n\nRevise the recipe to answer the verdict, keeping everything it did not fault. Reply with ONLY the whole revised JSON object.`
        : `# WORKED EXAMPLES (real recipes from this repo, other species)\n${examples(slug)}\n\n# PROSE\n${src.text}\n\nWrite the recipe for this plant. Reply with ONLY one JSON object.`;
    console.log(`[${round}/${rounds}] ${recipe ? 'revising' : 'drafting'} (${model})`);
    let draft;
    try {
        const r = claude(ask, CONTRACT, '');
        draft = r.json;
        cost += r.cost;
    } catch (e) {
        console.error(`✗ drafter: ${e.message}`);
        process.exit(1);
    }
    if (!draft.name) draft.name = slug;
    if (seedFlag != null) draft.seed = parseInt(seedFlag, 10) || 1;
    if (draft.seed == null) draft.seed = 1;

    // Grow it; a recipe that will not grow goes back with the error.
    const candidate = path.join(workDir, `round-${round}.grow.json`);
    writeFileSync(candidate, JSON.stringify(draft, null, 1) + '\n');
    let grown = grow(candidate, workDir);
    let tries = 0;
    while (!grown.ok && tries < 2) {
        tries++;
        console.log(`      would not grow — asking for a fix (${tries}/2)`);
        const fix = claude(`# PROSE\n${src.text}\n\n# RECIPE\n${JSON.stringify(draft, null, 1)}\n\n# THE GROWER SAID\n${grown.log.trim()}\n\nFix the recipe so it grows. Reply with ONLY the whole JSON object.`, CONTRACT, '');
        cost += fix.cost;
        draft = fix.json;
        if (!draft.name) draft.name = slug;
        if (draft.seed == null) draft.seed = 1;
        writeFileSync(candidate, JSON.stringify(draft, null, 1) + '\n');
        grown = grow(candidate, workDir);
    }
    if (!grown.ok) {
        console.error(`✗ the recipe would not grow:\n${grown.log}`);
        process.exit(1);
    }
    recipe = draft;
    const line = grown.log.split('\n').find((l) => l.startsWith('✓')) || '';
    console.log(`      ${line.trim()}`);
    writeFileSync(outPath, JSON.stringify(recipe, null, 1) + '\n');

    if (noJudge || round === rounds) break;

    // The turntable decides.
    console.log(`      judging against the prose`);
    const verdictAsk = `# PROSE\n${src.text}\n\n# RECIPE\n${JSON.stringify(recipe, null, 1)}\n\n# SHEETS\nLook at each of these with the Read tool:\n${grown.sheets.map((s) => `- ${s}`).join('\n')}\n(The first is a 3-view turntable of the grown plant; the second is the same plant at six ages, one scale, youngest left; the third is the same plant through the year, bud to bare, left to right.)\n\nDoes what grew match the prose — silhouette, proportions, crown, colour, bark, what it bears, how it dies if the prose says? Judge the picture, not the numbers. Reply with ONLY one JSON object: {"verdict":"accept"|"revise","notes":"what matches and what does not, in one short paragraph","changes":{…only the recipe fields to change, with their new values…}}`;
    let verdict;
    try {
        const r = claude(verdictAsk, 'You are the turntable: the last word on whether a grown plant matches the prose it came from. Be concrete and sparing — name the two or three things that most need to change, and leave alone what already reads right. Never invent lore the prose does not contain. Know what the sheets can and cannot show: fruit and blooms are sockets, and appear ONLY on the turntable (the first sheet), as plain white spheres hung at them — stand-ins for whatever the world hangs there — so judge their placement and number there, never their look, and never fault the life or year sheets for showing none; glow is washed towards white by the previewer, so judge colour, not brightness; bark detail is fine-scale and reads as tone at this distance.', 'Read');
        verdict = r.json;
        cost += r.cost;
    } catch (e) {
        console.error(`⚠ judge: ${e.message} — keeping the draft`);
        break;
    }
    console.log(`      ${verdict.verdict === 'accept' ? '✓ accepted' : '↻ revise'}: ${verdict.notes || ''}`);
    if (verdict.verdict === 'accept') break;
    feedback = `${verdict.notes || ''}\nSuggested changes: ${JSON.stringify(verdict.changes || {})}`;
    if (verdict.changes && typeof verdict.changes === 'object') Object.assign(recipe, verdict.changes);
}

// Final proof sheets from the recipe as saved.
const final = grow(outPath, workDir);
console.log(`\n✓ ${path.relative(ROOT, outPath)}`);
for (const s of final.sheets || []) console.log(`  ${path.relative(ROOT, s)}`);
console.log(`  drafted on the subscription (${cost.toFixed(3)} usd-equivalent)`);
