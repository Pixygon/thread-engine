# thread-engine

<!-- pixygon:workflow-start v2 -->
## Working in this repo (MANDATORY, short — the long form is linked)

1. **Start:** run `pearl preflight` from the repo root and begin your first reply with the `✦ Preflight <token> — …` line it prints. That line is your proof of onboarding.
2. **Memory:** before any investigation or architecture claim, read `/home/pixygon/.claude/projects/-home-pixygon-repos/memory/MEMORY.md` and open the files whose descriptions match your task; grep `MEMORY-ARCHIVE.md` there before re-auditing a site or a ship failure. Write durable findings back as one file per fact (frontmatter: name, description, metadata.type) plus one index line; never duplicate.
3. **UI work:** read the design canon `/home/pixygon/repos/pixygon-packages/@pixygon/design/tokens.mjs` first and stay in THIS site's brand voice. Mock → founder's yes → implement → screenshot against the real build.
4. **Web/SPA:** crawlers and answer engines must get real HTML, never an empty root div. Use `@pixygon/seo` (prerender in a jammy Playwright stage, `<PixygonSEO>`), never a hand-rolled prerender; verify with `curl -A Googlebot <url>`.
5. **Finish:** `pearl ship` from the repo root is THE last step — never commit by hand or call the release endpoints yourself. It runs the tests (the visual judge only when a UI file changed), drafts the changelog, releases on the API, commits and pushes. Flags you will need: `--review`, `--no-test`, `--dry`. Log its output to a file; never pipe it. If `pearl` is not on PATH: `node /home/pixygon/repos/Dyson/scripts/pearl.mjs ship`.
6. **Fiction or names:** the Universe Codex is canon. `pearl codex corpus` primes you; `pearl codex search` before inventing anything; `pearl codex push <file.md>` to add. Format and tiers: [`Dyson/docs/CODEX.md`](../Dyson/docs/CODEX.md).

Full spec: [`Dyson/docs/WORKFLOW.md`](../Dyson/docs/WORKFLOW.md) · what `pearl ship` does step by step: [`Dyson/docs/WORKFLOW.md#ship`](../Dyson/docs/WORKFLOW.md) · system map: [`Dyson/docs/workflows/registry.yml`](../Dyson/docs/workflows/registry.yml).

**Project ID**: `<unknown — set via PixygonServer admin or in this repo's .pixygon.json>`

Full spec: [`Dyson/docs/WORKFLOW.md`](../Dyson/docs/WORKFLOW.md). System map:
[`Dyson/docs/workflows/registry.yml`](../Dyson/docs/workflows/registry.yml) — also
rendered as the Atlas tab in Dyson.

_This section is managed by `Dyson/scripts/standardize-repos.mjs`. Don't edit
by hand — change the script or the canonical doc instead._
<!-- pixygon:workflow-end -->

