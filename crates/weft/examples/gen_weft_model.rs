//! Writes **weft-model** — the Thread's modeling library — as a weftpack.
//!
//! The library itself lives in [`weft_lang::model_lib`] (so it is testable and
//! the browser/CLI can carry it built-in); this example is how it becomes a
//! *package*: content-addressed defs, petname exports, publishable to wpm.
//!
//! ```sh
//! cargo run -p weft --example gen_weft_model
//! weftpack publish packages/weft-model/weft-model.weftpack.json
//! ```

fn main() {
    let pkg = weft_lang::model_lib::package();
    pkg.verify().expect("the library verifies");
    let dir = std::path::Path::new("packages/weft-model");
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("weft-model.weftpack.json");
    let json = serde_json::to_string_pretty(&pkg).expect("serializes");
    std::fs::write(&path, &json).expect("writes");
    println!(
        "wrote {} — {} exports, {} defs, {:.0} KB",
        path.display(),
        pkg.exports.len(),
        pkg.defs.len(),
        json.len() as f32 / 1024.0
    );
    for (petname, hash) in &pkg.exports {
        println!("  {petname:<12} {hash}");
    }
}
