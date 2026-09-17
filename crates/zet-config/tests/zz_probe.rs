//! Temporary probe. Deleted after the hunt.

use std::path::{Path, PathBuf};

use zet_config::{repaired, Config, load, save};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn probe_nan_font_size() {
    let dir = scratch("probe-nan-size");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[font]\nsize = nan\n").expect("write");
    let loaded = load(&path).expect("loads");
    println!("diagnostics: {:?}", loaded.diagnostics);
    println!("loaded size = {}", loaded.config.font.size);
    println!("loaded size is_finite = {}", loaded.config.font.size.is_finite());
    println!(
        "loaded size clamped to the panel's range = {}",
        loaded.config.font.size.clamp(4.0, 72.0)
    );
    let fixed = repaired(&loaded.config);
    println!("repaired size = {}", fixed.font.size);
}

#[test]
fn probe_nan_text_scale() {
    let dir = scratch("probe-nan-scale");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[appearance]\ntext-scale = nan\n").expect("write");
    let loaded = load(&path).expect("loads");
    println!("diagnostics: {:?}", loaded.diagnostics);
    println!("repaired text-scale = {}", repaired(&loaded.config).appearance.text_scale);
    println!("grid size would be {}", 13.0f32 * loaded.config.appearance.text_scale);
}

#[test]
fn probe_inf_font_size() {
    let dir = scratch("probe-inf-size");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[font]\nsize = inf\n").expect("write");
    let loaded = load(&path).expect("loads");
    println!("diagnostics: {:?}", loaded.diagnostics);
    println!("loaded size = {}", loaded.config.font.size);
}

#[test]
fn probe_save_write_is_direct() {
    // Not a test of behaviour, just evidence of what `save` does to an existing file.
    let dir = scratch("probe-save");
    let path = dir.join("config.toml");
    let mut config = Config::default();
    config.font.size = 14.0;
    save(&config, &path).expect("saves");
    let metadata = std::fs::metadata(&path).expect("metadata");
    println!("config.toml is {} bytes", metadata.len());
}
