//! Temporary probe: prints the cell metrics for the default face.

use zet_font::{FontLibrary, Metrics, Weight};

#[test]
fn probe() {
    let mut library = FontLibrary::new();
    for name in ["Cascadia Mono", "Consolas"] {
        let face = match library.resolve(name, Weight::NORMAL, false) {
            Some(face) => face,
            None => {
                println!("{name}: not installed");
                continue;
            }
        };
        for ppem in [13.0f32, 16.0, 26.0, 13.0 * 1.25] {
            let m = Metrics::from_face(&face, ppem).expect("metrics");
            println!(
                "{name} @ {ppem}: cell={}x{} baseline={} underline_top={} strikeout_top={} thickness={}",
                m.cell_width,
                m.cell_height,
                m.baseline,
                m.underline_top,
                m.strikeout_top,
                m.underline_thickness
            );
            println!(
                "   fracs: baseline={} underline_top={} strikeout_top={}",
                m.baseline.fract(),
                m.underline_top.fract(),
                m.strikeout_top.fract()
            );
        }
    }
}
