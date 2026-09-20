# weft-lang — the Weft v0.1 reference implementation

Weft is the Thread's native code format: a typed, **total**, effect-explicit,
content-addressed program graph (spec: `thread-spec/specs/weft-v0.1.md`). This
crate is the reference implementation — canonical encoding and hashing, the
verifier (types, transitive effect rows, static fuel bounds, contracts), the
deterministic interpreter, packages (`pack`), the audit projection
(`project`), and since 2026-09-20 the **`.weft` text surface** (`text`): a
small syntax an agent can write and a pack can be rendered back into.

The command line lives in `crates/weft-cli` and is called `weft`:

```text
weft verify <pack.json>                          certificate as JSON: effects, fuel bound, contracts per def
weft run    <pack.json> --fn <name|weft:hash> --input '<json args>' [--input-file f] [--fuel N]
                                                 deterministic evaluation → {value, fuel, fuel_bound, elapsed_us}
                                                 exit 2 + the violation as JSON when a contract fails
weft fmt    <pack.json>                          the .weft textual projection
weft parse  <file.weft> [-o out.weftpack.json] [--unverified]
                                                 text → canonical pack JSON (refused unless it verifies)
```

One JSON document on stdout; exit `0` ok, `1` any error, `2` contract violation.

## The `.weft` text surface

Weft has no source text: the graph is the ground truth, names are metadata.
`.weft` is a *projection* — whatever you write, `weft parse` turns into the
canonical graph, and `weft fmt` renders any graph back. Two guarantees, tested
on every package in this repo, the wpm seeds and the API's pilot:

- **`parse ∘ fmt = id`** on packages: rendering a pack to text and parsing it
  back yields byte-identical pack JSON (same hashes — the text loses nothing).
- **`fmt ∘ parse = id`** on canonical text: what `fmt` prints, `fmt(parse(·))`
  prints again, character for character.

Hand-written text becomes canonical after one `fmt`: parameter and binder
names are not in the graph, so `fmt` regenerates them (`a b c …` by depth,
`result` inside `ensures`); comments are dropped; layout is deterministic.
Definition names survive only as **export petnames** — an unexported
definition renders as `_<first 16 hex of its hash>`.

### Grammar

```ebnf
file      := "package" (ident | string) item*
item      := "export" "alias" ident "=" ident
           | ["export"] "def" ident "(" [param {"," param}] ")" "->" type
             ["effects" effect {"," effect}]
             ["requires" expr] ["ensures" expr]          (* either order, at most one each *)
             "=" expr
param     := ident ":" type

type      := "Int" | "Bool" | "Text" | "Fix" | "Action"
           | "List" type
           | "{" [label ":" type {"," label ":" type}] "}"   (* fields sort by name *)

expr      := "let" ident "=" expr "in" expr
           | "if" expr "then" expr "else" expr
           | binary
binary    := unary { binop unary }                            (* precedence climbing, table below *)
unary     := "not" comparison-or-tighter | postfix
postfix   := atom { "." label }
atom      := int | fix | string | "true" | "false"
           | "-" (int | fix)                                  (* negative literal *)
           | ident                                            (* a parameter or let binder *)
           | "(" expr ")"
           | "[" [expr {"," expr}] "]"                        (* list; [] is List Action *)
           | "{" [label ":" expr {"," label ":" expr}] "}"    (* record *)
           | ident "(" [expr {"," expr}] ")"                  (* call of a definition, by name *)
           | primfn "(" expr {"," expr} ")"
           | effect "{" [label ":" expr {"," label ":" expr}] "}"
           | "map"  "[" int "]" "(" expr "," ident "->" expr ")"
           | "fold" "[" int "]" "(" expr "," expr "," ident ident "->" expr ")"
           | "iota" "[" int "]" "(" expr ")"

primfn    := "text" | "fix_text" | "fix" | "trunc" | "len" | "sin" | "cos" | "cat"
effect    := "notify" | "navigate" | "codex_open" | "commerce_buy" | "presence_emit"
           | "set_state" | "give_item" | "despawn" | "spawn"

ident     := [A-Za-z_][A-Za-z0-9_]* ( "-" [A-Za-z_][A-Za-z0-9_]* )*   (* ring-point is one name; a - b is a subtraction *)
label     := [A-Za-z_][A-Za-z0-9_]*                            (* record/effect field names; may be any word *)
int       := [0-9]+                                           (* i64 *)
fix       := [0-9]+ "." [0-9]{1,6}                            (* millionths; 1.5 is Fix(1_500_000) *)
string    := '"' ( char | "\\" | '\"' | "\n" | "\r" | "\t" | "\u{hex}" )* '"'
comment   := "//" … end of line
```

Whitespace and newlines are insignificant. Keywords (`package export alias
def effects requires ensures let in if then else map fold iota true false and
or not result`), type names, prim functions and effect names are reserved and
cannot name a definition, parameter or binder; they can still be record
field labels.

**Operators** (higher binds tighter; `fmt` prints only the parentheses the
table requires):

| prec | operators | assoc | types |
|---|---|---|---|
| 1 | `or` | left | Bool |
| 2 | `and` | left | Bool |
| 3 | `not` (prefix) | — | Bool |
| 4 | `<` `<=` `==` (Int) · `<.` `<=.` `==.` (Fix) · `==$` (Text) | none — chains are an error | → Bool |
| 5 | `++` | left | Text |
| 6 | `+` `-` (Int) · `+.` `-.` (Fix) | left | |
| 7 | `*` `/` (Int) · `*.` `/.` (Fix) | left | |

Prim functions: `text(Int)→Text`, `fix_text(Fix)→Text`, `fix(Int)→Fix`,
`trunc(Fix)→Int`, `len(List T)→Int`, `sin(Fix)→Fix`, `cos(Fix)→Fix`,
`cat(List T, List T)→List T`. `/` and `/.` by zero are `0` (Weft has no traps).

**Semantics of the surface.** Parameters and `let`/`map`/`fold` binders are
names for de Bruijn indices; the innermost binding wins. A call names another
definition in the same file, in any order — the parser resolves the call
graph and hashes callees first; a cycle is refused (`recursion is not
expressible in Weft v0.1`). Inside `ensures`, `result` is the definition's
result. `effects` declares the row the verifier will check the body against.
The parser produces the graph and nothing more: type errors, undeclared
effects and contract typing are the **verifier's** findings (`weft parse`
runs it and refuses to write an unverifiable pack unless `--unverified`).

### Example 1 — a contract-carrying function

```weft
package clamp

export def clamp(x: Int, lo: Int, hi: Int) -> Int
  requires lo <= hi
  ensures lo <= result and result <= hi
  = if x < lo then lo else if hi < x then hi else x
```

```sh
weft parse clamp.weft -o clamp.weftpack.json
weft run clamp.weftpack.json --fn clamp --input '[10, 0, 5]'
# {"ok":true,"fn":"clamp","value":5,"fuel":…,"fuel_bound":…,"elapsed_us":…}
weft run clamp.weftpack.json --fn clamp --input '[1, 5, 0]'
# exit 2: {"ok":false,"stage":"contract","violation":{"which":"pre",…}}
```

### Example 2 — a behavior with effects and bounded iteration

The Meadow's chop-a-tree interaction, `(state, event) → {state, actions}`,
plus a ring of motes placed with `map` over `iota`:

```weft
package meadow

export def ring-point(i: Int, n: Int, radius: Fix) -> {x: Fix, y: Fix, z: Fix}
  = let theta = fix(i) /. fix(n) *. 6.283185 in
    {x: radius *. cos(theta), y: 0.0, z: radius *. sin(theta)}

export def ring(n: Int, radius: Fix) -> List {x: Fix, y: Fix, z: Fix}
  = map[256](iota[256](n), i -> ring-point(i, n, radius))

export def chop(state: {hits: Int}, event: {}) -> {actions: List Action, state: {hits: Int}}
  effects give_item, notify, despawn, spawn
  = let hits = state.hits + 1 in
    if 3 <= hits then
      {
        actions: cat(
          [give_item{item: 20100001, count: 3}, notify{text: "The tree falls."}, despawn{}],
          map[16](ring(6, 1.5), p -> spawn{builtin: "sphere", x: p.x, y: p.y +. 1.0, z: p.z, scale: 0.1}),
        ),
        state: {hits: 0},
      }
    else
      {actions: [], state: {hits: hits}}
```

`weft verify` reports `chop`'s effect row as exactly the set {`notify`,
`give_item`, `despawn`, `spawn`} and a static fuel bound covering the
256-element ring and the 16-element map; a `commerce_buy{…}` anywhere in the
body would be refused at verification, not at run time.

## Layout of the crate

| module | what |
|---|---|
| `lib.rs` | `Ty`, `Term`, `Def`, `Module`; canonical bytes + `hash_def`; `verify_module`; `eval_call` |
| `pack` | `Package` (`name`, `exports` petname→hash, `defs`), `verify`, `link` |
| `text` | `fmt` / `parse` — the `.weft` surface described above |
| `json` | typed JSON ↔ `Value` for the CLI/bridge seam |
| `project` | the older audit projection (hash-cited, not parseable) |
| `model_lib`, `draft_lib` | the modeling and drafting libraries, as packages |
