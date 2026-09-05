//! Render terrain previews to PNG — the iteration loop for Terra.
//! `cargo run -p thread-chisel --example terra_preview -- <outdir>`
use thread_chisel::terrain::{generate, preview, Relief, TerrainRecipe};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let size = 512usize;
    for (name, relief, cell, lat, hum) in [
        ("plains", Relief::Plains, 6.0, 0.45, 0.60),
        ("hills", Relief::Hills, 6.0, 0.42, 0.65),
        ("alpine", Relief::Alpine, 10.0, 0.55, 0.70),
        ("badlands", Relief::Badlands, 4.0, 0.30, 0.18),
        ("archipelago", Relief::Archipelago, 12.0, 0.25, 0.85),
    ] {
        let r = TerrainRecipe { relief, cell_size: cell, latitude: lat, humidity: hum, ..Default::default() };
        let t = std::time::Instant::now();
        let f = generate(&r, size, 0.0, 0.0);
        let ms = t.elapsed().as_millis();
        let px = preview(&f, &r);
        let path = format!("{out}/terra-{name}.png");
        image::save_buffer(&path, &px, size as u32, size as u32, image::ColorType::Rgb8)
            .expect("write png");
        let lo = f.height.iter().cloned().fold(f32::MAX, f32::min);
        let hi = f.height.iter().cloned().fold(f32::MIN, f32::max);
        println!("{name:12} {ms:>5}ms  relief {:>7.0}m..{:>7.0}m", lo, hi);
    }
}
