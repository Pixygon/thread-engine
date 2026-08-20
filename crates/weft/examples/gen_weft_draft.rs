//! Writes **weft-draft** — the level-design library — as a weftpack.
//!
//! ```sh
//! cargo run -p weft --example gen_weft_draft
//! weftpack publish packages/weft-draft/weft-draft.weftpack.json
//! ```

fn main() {
    let pkg = weft_lang::draft_lib::package();
    pkg.verify().expect("the drafting library verifies");
    let dir = std::path::Path::new("packages/weft-draft");
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("weft-draft.weftpack.json");
    let json = serde_json::to_string_pretty(&pkg).expect("serializes");
    std::fs::write(&path, &json).expect("writes");
    println!(
        "wrote {} — {} exports, {} defs, {:.0} KB",
        path.display(),
        pkg.exports.len(),
        pkg.defs.len(),
        json.len() as f32 / 1024.0
    );
}
