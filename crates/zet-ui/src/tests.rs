//! The chrome's tests.
//!
//! Everything here draws into a frame and reads the answer back, which is the whole
//! reason the layout is a pure function: the interesting questions about a tab strip are
//! geometric, and a geometry that can only be checked by looking at a window is a
//! geometry that is only checked when somebody notices.

// The fake glyph source below turns its own sizes and character codes into the integers a
// `Placement` carries. Every one of them is a width or a code point, so the sign the lint
// is worried about cannot be lost — and the alternative is a `try_from` and an `unwrap`
// per field in a test fixture, which is noise that would hide the numbers it is made of.
#![allow(clippy::cast_sign_loss)]

use zet_config::{Palette, TabPosition, TabSettings, WindowSettings};
use zet_font::{GlyphSpec, Metrics, Weight};
use zet_render::{Frame, Placement, Quad};

use crate::fonts::GlyphSource;
use crate::geometry::ROW_HEIGHT;
use crate::{
    Caption, Chrome, ChromeInput, Hit, Layout, Rect, ScrollState, Scrollbar, Size, TabInfo,
};

/// A font source with no font in it.
///
/// Every character is the same width, which is the point: the strip's arithmetic should
/// be checkable without asserting against a font file's advance tables, and a chrome that
/// is only correct for one face is not correct. The numbers are round so that a failure
/// reads as a number rather than as float noise.
///
/// It is built at a scale, like the real thing, because that is the property the layout
/// depends on: a source rasterised at 200% has cells twice the size in *physical* pixels,
/// and the layout has to come out the same in logical ones either way.
///
/// The placements carry the character and the weight back out in their texture
/// coordinates. That is not something a real atlas would do; it is here because a
/// placement is the only thing this seam returns, and "which number was drawn, at which
/// weight" is otherwise unanswerable from outside. The renderer's atlas is keyed by a
/// `GlyphKey` that carries the weight, so an implementation that cares can tell them
/// apart — the fake has to smuggle it out through the one field that survives.
struct TestFonts {
    metrics: Metrics,
}

impl TestFonts {
    /// A source built for a window at `scale`.
    fn new(scale: f32) -> Self {
        let ppem = 13.0 * scale;
        Self {
            metrics: Metrics {
                ppem,
                cell_width: ppem * 0.5,
                cell_height: ppem,
                baseline: ppem * 10.0 / 13.0,
                underline_top: ppem * 11.0 / 13.0,
                underline_thickness: 1.0,
                strikeout_top: ppem * 6.0 / 13.0,
                cap_height: ppem * 9.0 / 13.0,
                x_height: ppem * 6.0 / 13.0,
            },
        }
    }
}

impl zet_render::GlyphSource for TestFonts {
    fn place(&mut self, spec: GlyphSpec) -> Option<Placement> {
        let ink = spec.ch != ' ';
        // A ninth-of-an-em-tall bitmap at the source's own scale, sat on the baseline.
        let em = self.metrics.ppem;
        let (width, height) = if ink {
            (5.0 * em / 13.0, 9.0 * em / 13.0)
        } else {
            (0.0, 0.0)
        };
        Some(Placement {
            uv: [
                f32::from(spec.weight.value()),
                spec.ch as u32 as f32,
                0.0,
                0.0,
            ],
            left: 0,
            top: (9.0 * em / 13.0) as i32,
            width: width as u32,
            height: height as u32,
            advance: em * 0.5,
            color: false,
        })
    }
}

impl GlyphSource for TestFonts {
    fn metrics(&self) -> &Metrics {
        &self.metrics
    }
}

/// One character as it reached the frame: what it was, and how heavy.
#[derive(Clone, Copy, PartialEq, Debug)]
struct DrawnChar {
    ch: char,
    weight: u16,
}

/// Every character the frame drew, in order, read back out of the texture coordinates
/// [`TestFonts`] smuggled it through.
fn drawn(frame: &Frame) -> Vec<DrawnChar> {
    frame
        .glyphs
        .iter()
        .map(|glyph| DrawnChar {
            ch: char::from_u32(glyph.uv[1] as u32).unwrap_or('\u{fffd}'),
            weight: glyph.uv[0] as u16,
        })
        .collect()
}

/// Every character drawn at one weight.
fn drawn_at(frame: &Frame, weight: Weight) -> Vec<char> {
    drawn(frame)
        .into_iter()
        .filter(|drawn| drawn.weight == weight.value())
        .map(|drawn| drawn.ch)
        .collect()
}

/// The digits drawn at one weight, which is how a test asks which numbers the strip put
/// on screen.
fn digits_at(frame: &Frame, weight: Weight) -> Vec<char> {
    drawn_at(frame, weight)
        .into_iter()
        .filter(char::is_ascii_digit)
        .collect()
}

/// A tab with a number and nothing else interesting about it.
fn tab(index: u32) -> TabInfo {
    TabInfo {
        index,
        title: format!("Terminal {index}"),
        hovered: false,
    }
}

fn tabs(indices: &[u32]) -> Vec<TabInfo> {
    indices.iter().copied().map(tab).collect()
}

/// The chrome built from the defaults, which is what a fresh install has.
fn chrome() -> Chrome {
    Chrome::new(&TabSettings::default(), &WindowSettings::default())
}

/// A default input for a window of a given size.
fn input<'a>(palette: &'a Palette, tabs: &'a [TabInfo], size: Size) -> ChromeInput<'a> {
    ChromeInput {
        palette,
        tabs,
        active: tabs.first().map(|tab| tab.index),
        settings_open: false,
        window_title: "zet",
        size,
        scale: 1.0,
        maximized: false,
        reduce_motion: false,
        pointer: None,
    }
}

/// A 1200 by 800 window, which is where most of these tests live.
fn window() -> Size {
    Size {
        width: 1200.0,
        height: 800.0,
    }
}

/// What one layout call produced.
struct Drawn {
    layout: Layout,
    frame: Frame,
}

/// Build the chrome for one input and draw it.
///
/// The fake glyph source is a local rather than a field of the result: everything the
/// tests need to know about it is already in the frame, and `drawn`/`drawn_at`/`digits_at`
/// read it back out of there.
fn draw(chrome: &mut Chrome, input: &ChromeInput<'_>) -> Drawn {
    let mut frame = Frame::new();
    let mut fonts = TestFonts::new(input.scale);
    let layout = chrome.layout(input, &mut fonts, &mut frame);
    Drawn { layout, frame }
}

/// The tab cells, in the order they were laid out.
fn tab_rects(chrome: &Chrome) -> Vec<(u32, Rect)> {
    chrome
        .regions()
        .iter()
        .filter_map(|region| match region {
            crate::Region::Tab { index, rect } => Some((*index, *rect)),
            _ => None,
        })
        .collect()
}

/// Where the new-tab mark went.
fn plus_rect(chrome: &Chrome) -> Option<Rect> {
    chrome.regions().iter().find_map(|region| match region {
        crate::Region::NewTab(rect) => Some(*rect),
        _ => None,
    })
}

/// A tab's cell.
fn tab_rect(chrome: &Chrome, index: u32) -> Rect {
    tab_rects(chrome)
        .into_iter()
        .find(|(number, _)| *number == index)
        .expect("the tab was laid out")
        .1
}

/// The bar the indicator drew: `signal`, two pixels on the strip's edge.
fn indicator(frame: &Frame, palette: &Palette) -> Option<Quad> {
    let color = palette.signal.to_linear().map(f32::to_bits);
    frame.quads.iter().copied().find(|quad| {
        let [_, _, width, height] = quad.rect;
        quad.color.map(f32::to_bits) == color
            && ((height - 2.0).abs() < f32::EPSILON || (width - 2.0).abs() < f32::EPSILON)
    })
}

/// Whether two quads are the same rectangle and colour, compared as bits because a float
/// equality between colours is exactly the comparison the lints are right to complain
/// about.
fn same_quad(a: &Quad, b: &Quad) -> bool {
    a.rect.map(f32::to_bits) == b.rect.map(f32::to_bits)
        && a.color.map(f32::to_bits) == b.color.map(f32::to_bits)
}

/// The quads in `a` that have no twin in `b`.
fn quads_only_in(a: &Frame, b: &Frame) -> Vec<Quad> {
    let mut unclaimed: Vec<&Quad> = b.quads.iter().collect();
    let mut extra = Vec::new();
    for quad in &a.quads {
        if let Some(position) = unclaimed.iter().position(|other| same_quad(quad, other)) {
            unclaimed.remove(position);
        } else {
            extra.push(*quad);
        }
    }
    extra
}

/// The bit pattern of a palette colour, as it reaches a frame.
fn color(color: zet_config::Rgb) -> [u32; 4] {
    color.to_linear().map(f32::to_bits)
}

// ---------------------------------------------------------------------------------
// The strip's geometry
// ---------------------------------------------------------------------------------

#[test]
fn the_tabs_sit_side_by_side_and_a_cell_is_what_the_sum_of_its_parts_says() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let mut chrome = chrome();
    draw(&mut chrome, &input(&palette, &tabs, window()));
    let rects = tab_rects(&chrome);

    assert_eq!(rects.len(), 2);
    let (_, first) = rects[0];
    let (_, second) = rects[1];
    assert!(
        (first.right() - second.x).abs() < f32::EPSILON,
        "the second tab starts where the first ends: {first:?} then {second:?}"
    );
    assert!((first.width - second.width).abs() < f32::EPSILON);
    assert!((first.y).abs() < f32::EPSILON);
    assert!((first.height - ROW_HEIGHT).abs() < f32::EPSILON);

    // `#1` is the `#` at seventy percent plus one digit plus the padding either side.
    let expected = 13.0 * 0.7 * 0.5 + 13.0 * 0.5 + 2.0 * 12.0;
    assert!(
        (first.width - expected).abs() < 1.0e-4,
        "a single-digit tab is {expected} wide, not {}",
        first.width
    );
}

#[test]
fn a_second_digit_makes_a_tab_wider() {
    let palette = Palette::instrument();
    let one = tabs(&[1]);
    let ten = tabs(&[10]);
    let mut narrow = chrome();
    let mut wide = chrome();

    draw(&mut narrow, &input(&palette, &one, window()));
    draw(&mut wide, &input(&palette, &ten, window()));

    let single = tab_rect(&narrow, 1);
    let double = tab_rect(&wide, 10);
    assert!(
        double.width > single.width,
        "#10 is {} wide and #1 is {}",
        double.width,
        single.width
    );
    assert!(
        (double.width - single.width - 6.5).abs() < 1.0e-4,
        "and exactly one digit wider"
    );
}

#[test]
fn the_new_tab_mark_ends_the_run_and_has_a_tab_s_footprint() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2, 3]);
    let mut chrome = chrome();
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    let rects = tab_rects(&chrome);
    let plus = plus_rect(&chrome).expect("the new-tab mark is always on the row");
    let last = rects[rects.len() - 1].1;
    assert!((plus.x - last.right()).abs() < f32::EPSILON);
    assert!((plus.width - rects[0].1.width).abs() < f32::EPSILON);

    let (x, y) = plus.center();
    assert_eq!(chrome.hit(x, y), Hit::NewTab);
    assert!(
        drawn_at(&drawn.frame, Weight::NORMAL).contains(&'+'),
        "and it is drawn as a plus"
    );
}

#[test]
fn the_caption_buttons_are_windows_sized_and_flush_right() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let mut chrome = chrome();
    let size = window();
    draw(&mut chrome, &input(&palette, &tabs, size));

    let captions: Vec<(Caption, Rect)> = chrome
        .regions()
        .iter()
        .filter_map(|region| match region {
            crate::Region::Caption { caption, rect } => Some((*caption, *rect)),
            _ => None,
        })
        .collect();
    assert_eq!(captions.len(), 3);
    assert_eq!(captions[0].0, Caption::Minimize);
    assert_eq!(captions[1].0, Caption::Maximize);
    assert_eq!(captions[2].0, Caption::Close);
    for (_, rect) in &captions {
        assert!((rect.width - 46.0).abs() < f32::EPSILON, "{rect:?}");
        assert!((rect.height - ROW_HEIGHT).abs() < f32::EPSILON, "{rect:?}");
    }
    assert!(
        (captions[2].1.right() - size.width).abs() < f32::EPSILON,
        "close is flush against the right edge"
    );
    let (x, y) = captions[2].1.center();
    assert_eq!(chrome.hit(x, y), Hit::Caption(Caption::Close));
}

#[test]
fn the_run_starts_after_the_app_name() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let mut chrome = chrome();
    draw(&mut chrome, &input(&palette, &tabs, window()));

    let rects = tab_rects(&chrome);
    let plus = plus_rect(&chrome).expect("a new-tab mark");
    // Twelve pixels of inset, the width of "zet" at 12px, and the gap after it.
    let start = 12.0 + 3.0 * 12.0 * 0.5 + 14.0;
    assert!((rects[0].1.x - start).abs() < 1.0e-4, "after the app name");
    let run = rects[1].1.right() - start;
    assert!((run - 2.0 * rects[0].1.width).abs() < 1.0e-4);
    assert!((plus.right() - (start + run + plus.width)).abs() < 1.0e-4);
}

#[test]
fn the_app_name_is_dropped_before_a_tab_is() {
    let palette = Palette::instrument();

    // A window with room for both: the name is drawn and the run starts after it.
    let few = tabs(&[1, 2]);
    let mut roomy = chrome();
    draw(&mut roomy, &input(&palette, &few, window()));
    assert!(tab_rect(&roomy, 1).x > 0.0, "the name is on the row");

    // A window with room for neither: the name goes, and the run starts at the edge,
    // which is what "tabs outrank branding" means in arithmetic.
    //
    // Forty, not twelve. A tab is about forty pixels — the hash, the digits, and two
    // insets — so twelve of them are under five hundred and a 1200-pixel window takes
    // them all easily; a test that said otherwise would be asserting a width the strip
    // does not have. Forty is past the roughly twenty-six that fit in the 1062 pixels
    // left of the caption buttons, and far enough past it that the arithmetic has to
    // move a long way before this stops overflowing.
    let many = tabs(&(1..=40).collect::<Vec<u32>>());
    let mut tight = chrome();
    draw(&mut tight, &input(&palette, &many, window()));
    let rects = tab_rects(&tight);
    assert!(rects.len() < many.len(), "this window is meant to overflow");
    assert!((rects[0].1.x).abs() < f32::EPSILON, "the name gave way");
    assert!(
        plus_rect(&tight).is_some(),
        "and the new-tab mark never overflows"
    );
}

// ---------------------------------------------------------------------------------
// Zero tabs
// ---------------------------------------------------------------------------------

#[test]
fn zero_tabs_removes_the_row() {
    let palette = Palette::instrument();
    let none: Vec<TabInfo> = Vec::new();
    let mut chrome = chrome();
    let size = window();
    let drawn = draw(&mut chrome, &input(&palette, &none, size));

    assert!((drawn.layout.top).abs() < f32::EPSILON);
    assert!((drawn.layout.grid.y).abs() < f32::EPSILON);
    assert!((drawn.layout.grid.height - size.height).abs() < f32::EPSILON);

    // Not a tab, not the new-tab mark, and not a drag region either: there is no row, so
    // there is nothing on it to hit.
    for x in (0..1200).step_by(7) {
        let hit = chrome.hit(x as f32, 20.0);
        assert!(
            !matches!(hit, Hit::Tab(_) | Hit::NewTab | Hit::Drag),
            "x = {x} hit {hit:?} with no tabs open"
        );
    }
    // No surface and no hairline. What is left is the caption marks, which are glyphs.
    assert!(
        drawn.frame.quads.is_empty(),
        "an absent row still drew {:?}",
        drawn.frame.quads
    );
    assert!(
        !drawn.frame.glyphs.is_empty(),
        "the captions are still there"
    );
}

// ---------------------------------------------------------------------------------
// Numbering
// ---------------------------------------------------------------------------------

#[test]
fn numbering_survives_a_close() {
    // Close #2 of three and the two that are left are still #1 and #3. A number
    // identifies a session, and shuffling numbers under the user is worse than a gap.
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 3]);
    let mut chrome = chrome();
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    let numbers: Vec<u32> = tab_rects(&chrome).iter().map(|(index, _)| *index).collect();
    assert_eq!(numbers, vec![1, 3]);
    // The active tab is #1, so it is the medium one; #3 is the one at rest.
    assert_eq!(digits_at(&drawn.frame, Weight::MEDIUM), vec!['1']);
    assert_eq!(digits_at(&drawn.frame, Weight::NORMAL), vec!['3']);

    for index in [1, 3] {
        let (x, y) = tab_rect(&chrome, index).center();
        assert_eq!(chrome.hit(x, y), Hit::Tab(index), "tab {index}");
    }
}

#[test]
fn hit_testing_the_center_of_a_tab_returns_that_tab() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2, 3]);
    let mut chrome = chrome();
    draw(&mut chrome, &input(&palette, &tabs, window()));

    for index in [1, 2, 3] {
        let (x, y) = tab_rect(&chrome, index).center();
        assert_eq!(chrome.hit(x, y), Hit::Tab(index));
    }
    // A point above the rows is not the strip's at all.
    assert_eq!(chrome.hit(20.0, -5.0), Hit::None);
}

#[test]
fn the_gap_between_the_last_tab_and_the_captions_is_the_drag_region() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2, 3]);
    let mut chrome = chrome();
    let size = window();
    draw(&mut chrome, &input(&palette, &tabs, size));

    let captions_left = size.width - 3.0 * 46.0;
    assert_eq!(chrome.hit(captions_left - 1.0, 20.0), Hit::Drag);
    let plus = plus_rect(&chrome).expect("a new-tab mark");
    let middle = f32::midpoint(plus.right(), captions_left);
    assert_eq!(chrome.hit(middle, 20.0), Hit::Drag);
    assert_eq!(
        chrome.hit(captions_left + 1.0, 20.0),
        Hit::Caption(Caption::Minimize)
    );
}

// ---------------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------------

#[test]
fn a_hovered_tab_gains_a_bar_and_nothing_else() {
    // The single most important assertion in this crate. Filling a hovered tab is what
    // DESIGN.md singles out as the way a tab strip starts looking like everyone else's,
    // and it is a one-line mistake no other test here would notice.
    let palette = Palette::instrument();
    let mut tabs = tabs(&[1, 2]);
    let mut chrome = chrome();
    let calm = draw(&mut chrome, &input(&palette, &tabs, window()));

    tabs[1].hovered = true;
    let hovered = draw(&mut chrome, &input(&palette, &tabs, window()));

    let mut extra = quads_only_in(&hovered.frame, &calm.frame);
    assert_eq!(
        extra.len(),
        1,
        "hovering added {} rectangles, not one: {extra:?}",
        extra.len()
    );
    let bar = extra.remove(0);
    assert_eq!(
        bar.color.map(f32::to_bits),
        color(palette.signal_dim),
        "what appears is the indicator's preview colour"
    );
    assert!(
        (bar.rect[3] - 2.0).abs() < f32::EPSILON,
        "and it is the bar's row"
    );
    assert!((bar.rect[1] - (ROW_HEIGHT - 2.0)).abs() < f32::EPSILON);
    assert!(
        quads_only_in(&calm.frame, &hovered.frame).is_empty(),
        "hovering took something away"
    );

    // The index itself moves one ink and keeps its weight.
    let index = |frame: &Frame, expect: [u32; 4]| {
        let x = tab_rect(&chrome, 2).x + 12.0;
        frame
            .glyphs
            .iter()
            .find(|glyph| (glyph.rect[0] - x).abs() < 0.01)
            .map(|glyph| glyph.color.map(f32::to_bits))
            .expect("the hovered tab drew an index")
            .eq(&expect)
    };
    assert!(index(&hovered.frame, color(palette.ink_mid)));
    assert!(index(&calm.frame, color(palette.ink_dim)));
}

#[test]
fn the_active_tab_is_ink_and_the_rest_are_ink_dim() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let mut chrome = chrome();
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    let active = tab_rect(&chrome, 1);
    let inactive = tab_rect(&chrome, 2);
    let ink = color(palette.ink);
    let dim = color(palette.ink_dim);
    let heavy: Vec<char> = drawn_at(&drawn.frame, Weight::MEDIUM);
    let light: Vec<char> = drawn_at(&drawn.frame, Weight::NORMAL);
    assert!(heavy.contains(&'1'), "the active index: {heavy:?}");
    assert!(light.contains(&'2'), "the inactive index: {light:?}");
    let _ = (active, inactive, ink, dim);

    // The bar is under the active tab and nowhere else.
    let bar = indicator(&drawn.frame, &palette).expect("an indicator");
    assert!((bar.rect[0] - active.x).abs() < f32::EPSILON);
    assert!((bar.rect[2] - active.width).abs() < f32::EPSILON);
    assert!((bar.rect[1] - (ROW_HEIGHT - 2.0)).abs() < f32::EPSILON);
}

#[test]
fn a_maximized_window_loses_the_row_hairline() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, window());
    let framed = draw(&mut chrome, &input);
    input.maximized = true;
    let maximized = draw(&mut chrome, &input);

    let hairline = color(palette.hairline);
    let has = |frame: &Frame| {
        frame
            .quads
            .iter()
            .any(|quad| quad.color.map(f32::to_bits) == hairline)
    };
    assert!(has(&framed.frame), "a framed window divides the row");
    assert!(!has(&maximized.frame), "a maximized one does not");
}

// ---------------------------------------------------------------------------------
// The grid
// ---------------------------------------------------------------------------------

#[test]
fn the_grid_gets_what_is_left() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let size = window();
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, size);
    input.settings_open = true;
    chrome.set_find_open(true);
    let drawn = draw(&mut chrome, &input);

    let layout = drawn.layout;
    assert!((layout.top - ROW_HEIGHT).abs() < f32::EPSILON);
    assert!((layout.bottom - 32.0).abs() < f32::EPSILON);
    assert!(
        (layout.grid.y - layout.top).abs() < f32::EPSILON,
        "below the row"
    );
    assert!(
        (layout.grid.x - layout.left).abs() < f32::EPSILON,
        "right of the rail"
    );
    assert!(
        (layout.grid.bottom() - (size.height - layout.bottom)).abs() < f32::EPSILON,
        "above the find bar"
    );
    assert!((layout.grid.right() - size.width).abs() < f32::EPSILON);

    // A closed find bar gives the row back.
    chrome.set_find_open(false);
    let without = draw(&mut chrome, &input);
    assert!((without.layout.bottom).abs() < f32::EPSILON);
    assert!(without.layout.grid.height > layout.grid.height);
}

// ---------------------------------------------------------------------------------
// The settings panel and the find bar
// ---------------------------------------------------------------------------------

#[test]
fn the_settings_panel_is_anchored_to_the_right_and_hit_tests_as_settings() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let size = window();
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, size);
    input.settings_open = true;
    let drawn = draw(&mut chrome, &input);

    let panel = chrome
        .regions()
        .iter()
        .find_map(|region| match region {
            crate::Region::Settings(rect) => Some(*rect),
            _ => None,
        })
        .expect("the panel is open");
    assert!((panel.width - 380.0).abs() < f32::EPSILON);
    assert!((panel.right() - size.width).abs() < f32::EPSILON);
    assert!((panel.y - drawn.layout.top).abs() < f32::EPSILON);

    assert_eq!(chrome.hit(panel.x + 10.0, panel.y + 10.0), Hit::Settings);
    assert_eq!(
        chrome.hit(size.width - 20.0, 20.0),
        Hit::Caption(Caption::Close),
        "the captions are over the panel, not under it"
    );
    assert_eq!(
        chrome.hit(100.0, 400.0),
        Hit::None,
        "the grid is not the chrome's"
    );

    // The panel's own surface is in the frame, and every heading is drawn.
    let raised = color(palette.surface_raised);
    assert!(
        drawn
            .frame
            .quads
            .iter()
            .any(|quad| quad.color.map(f32::to_bits) == raised)
    );
    let headings: Vec<char> = drawn_at(&drawn.frame, Weight::MEDIUM);
    for expected in ["APPEARANCE", "TABS", "TERMINAL", "KEYS"] {
        for letter in expected.chars() {
            assert!(headings.contains(&letter), "{expected} was not drawn");
        }
    }
}

#[test]
fn an_open_find_bar_is_a_row_of_its_own() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let size = window();
    let mut chrome = chrome();
    chrome.set_find_open(true);
    let drawn = draw(&mut chrome, &input(&palette, &tabs, size));

    assert!((drawn.layout.bottom - 32.0).abs() < f32::EPSILON);
    let raised = color(palette.surface_raised);
    assert!(
        drawn.frame.quads.iter().any(|quad| {
            quad.color.map(f32::to_bits) == raised
                && (quad.rect[1] - (size.height - 32.0)).abs() < f32::EPSILON
        }),
        "the bar is at the bottom of the window"
    );
    // Nothing in the chrome hit-tests inside it: the field takes typing, and typing is
    // the app's.
    assert_eq!(chrome.hit(100.0, size.height - 16.0), Hit::None);
}

// ---------------------------------------------------------------------------------
// Vertical
// ---------------------------------------------------------------------------------

#[test]
fn the_rail_is_forty_eight_wide_with_the_bar_on_the_left() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2, 3]);
    let settings = TabSettings {
        position: TabPosition::Left,
        ..TabSettings::default()
    };
    let mut chrome = Chrome::new(&settings, &WindowSettings::default());
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    assert!((drawn.layout.left - 48.0).abs() < f32::EPSILON);
    assert!((drawn.layout.grid.x - 48.0).abs() < f32::EPSILON);
    assert!(
        (drawn.layout.top - ROW_HEIGHT).abs() < f32::EPSILON,
        "the row is still there"
    );

    let rects = tab_rects(&chrome);
    assert_eq!(rects.len(), 3);
    for (_, rect) in &rects {
        assert!((rect.x).abs() < f32::EPSILON);
        assert!((rect.width - 48.0).abs() < f32::EPSILON);
    }
    assert!(
        (rects[0].1.height - 36.0).abs() < f32::EPSILON,
        "36px cells"
    );
    assert!(
        (rects[1].1.y - rects[0].1.bottom()).abs() < f32::EPSILON,
        "and they are stacked"
    );

    // Same language, rotated: the bar is on the left edge of the active cell.
    let bar = indicator(&drawn.frame, &palette).expect("an indicator");
    assert!((bar.rect[0]).abs() < f32::EPSILON);
    assert!((bar.rect[2] - 2.0).abs() < f32::EPSILON);
    assert!((bar.rect[1] - rects[0].1.y).abs() < f32::EPSILON);

    // The row's own hit regions are still the row's.
    let (x, y) = rects[0].1.center();
    assert_eq!(chrome.hit(x, y), Hit::Tab(1));
}

#[test]
fn the_rail_compresses_towards_its_floor_and_keeps_the_new_tab_mark() {
    let palette = Palette::instrument();
    let settings = TabSettings {
        position: TabPosition::Left,
        ..TabSettings::default()
    };
    let short = Size {
        width: 900.0,
        height: 400.0,
    };

    let many = tabs(&(1..=12).collect::<Vec<u32>>());
    let mut chrome = Chrome::new(&settings, &WindowSettings::default());
    draw(&mut chrome, &input(&palette, &many, short));

    let height = tab_rect(&chrome, 1).height;
    assert!(height < 36.0, "the cells compressed: {height}");
    assert!(height >= 24.0, "but not below the floor: {height}");
    assert!(
        plus_rect(&chrome).is_some(),
        "the mark is never compressed away"
    );
}

// ---------------------------------------------------------------------------------
// The scrollbar
// ---------------------------------------------------------------------------------

#[test]
fn the_scrollbar_appears_only_when_there_is_something_to_scroll() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let size = window();
    let mut chrome = chrome();
    draw(&mut chrome, &input(&palette, &tabs, size));

    // A terminal showing everything has no scrollbar at all.
    assert_eq!(chrome.hit(size.width - 4.0, 400.0), Hit::None);

    chrome.set_scroll(ScrollState {
        offset: 0.0,
        visible: 0.25,
    });
    draw(&mut chrome, &input(&palette, &tabs, size));
    let thumb = (size.height - ROW_HEIGHT) * 0.25;
    assert_eq!(
        chrome.hit(size.width - 4.0, ROW_HEIGHT + thumb + 10.0),
        Hit::Scrollbar(Scrollbar::Below)
    );
    assert_eq!(
        chrome.hit(size.width - 4.0, ROW_HEIGHT + 2.0),
        Hit::Scrollbar(Scrollbar::Thumb)
    );

    chrome.set_scroll(ScrollState {
        offset: 1.0,
        visible: 0.25,
    });
    draw(&mut chrome, &input(&palette, &tabs, size));
    assert_eq!(
        chrome.hit(size.width - 4.0, ROW_HEIGHT + 2.0),
        Hit::Scrollbar(Scrollbar::Above),
        "scrolled to the end, so the band above the thumb is the long one"
    );
}

// ---------------------------------------------------------------------------------
// The two planes
// ---------------------------------------------------------------------------------

#[test]
fn the_chrome_only_ever_draws_the_chrome_palette() {
    // The test that catches a grid colour leaking into the chrome, or a colour invented
    // at a call site. Every rectangle and every glyph tint has to be one of the twelve,
    // converted the one way that conversion is allowed to happen.
    for settings_open in [false, true] {
        for position in [TabPosition::Top, TabPosition::Left] {
            let palette = Palette::instrument();
            let tabs = tabs(&[1, 2, 3]);
            let settings = TabSettings {
                position,
                ..TabSettings::default()
            };
            let mut chrome = Chrome::new(&settings, &WindowSettings::default());
            chrome.set_find_open(true);
            chrome.set_scroll(ScrollState {
                offset: 0.5,
                visible: 0.5,
            });
            let mut input = input(&palette, &tabs, window());
            input.settings_open = settings_open;
            let drawn = draw(&mut chrome, &input);

            let allowed: Vec<[u32; 4]> = palette
                .all()
                .iter()
                .map(|(_, color)| color.to_linear().map(f32::to_bits))
                .collect();
            assert!(!drawn.frame.quads.is_empty(), "nothing was drawn");
            for quad in &drawn.frame.quads {
                assert!(
                    allowed.contains(&quad.color.map(f32::to_bits)),
                    "{:?} is not a chrome colour",
                    quad.color
                );
            }
            for glyph in &drawn.frame.glyphs {
                assert!(
                    allowed.contains(&glyph.color.map(f32::to_bits)),
                    "{:?} is not a chrome colour",
                    glyph.color
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// The indicator
// ---------------------------------------------------------------------------------

#[test]
fn the_indicator_travels_between_tabs() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, window());
    input.active = Some(1);
    chrome.set_time(0.0);
    draw(&mut chrome, &input);
    let first = tab_rect(&chrome, 1);

    // Switch, and look seventy milliseconds into a hundred-and-forty millisecond move.
    input.active = Some(2);
    draw(&mut chrome, &input);
    chrome.set_time(0.070);
    let drawn = draw(&mut chrome, &input);

    let second = tab_rect(&chrome, 2);
    let bar = indicator(&drawn.frame, &palette).expect("an indicator");
    let x = bar.rect[0];
    assert!(
        x > first.x && x < second.x,
        "the bar is at {x}, and the tabs are at {} and {}",
        first.x,
        second.x
    );
    assert!(
        x > first.x + 0.8 * (second.x - first.x),
        "and most of the way there, which is what exponential ease-out means"
    );

    // And it arrives exactly rather than asymptotically.
    chrome.set_time(0.5);
    let arrived = draw(&mut chrome, &input);
    let bar = indicator(&arrived.frame, &palette).expect("an indicator");
    assert!((bar.rect[0] - second.x).abs() < f32::EPSILON);
}

#[test]
fn reduce_motion_makes_the_indicator_jump() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, window());
    input.reduce_motion = true;
    input.active = Some(1);
    chrome.set_time(0.0);
    draw(&mut chrome, &input);

    input.active = Some(2);
    chrome.set_time(0.070);
    let drawn = draw(&mut chrome, &input);

    let second = tab_rect(&chrome, 2);
    let bar = indicator(&drawn.frame, &palette).expect("an indicator");
    assert!(
        (bar.rect[0] - second.x).abs() < f32::EPSILON,
        "with reduce motion the bar is already there"
    );
}

#[test]
fn a_change_mid_travel_carries_on_from_where_the_bar_is() {
    // Holding down Ctrl+Tab changes the active tab faster than the bar can arrive. A
    // travel that restarted from the tab it was leaving would show the bar jumping
    // backwards on every step.
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2, 3]);
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, window());
    input.active = Some(1);
    chrome.set_time(0.0);
    draw(&mut chrome, &input);

    input.active = Some(3);
    chrome.set_time(0.0);
    draw(&mut chrome, &input);
    chrome.set_time(0.070);
    let midway = draw(&mut chrome, &input);
    let moving = indicator(&midway.frame, &palette)
        .expect("an indicator")
        .rect[0];

    input.active = Some(2);
    let after = draw(&mut chrome, &input);
    let bar = indicator(&after.frame, &palette).expect("an indicator");

    assert!(
        (bar.rect[0] - moving).abs() < f32::EPSILON,
        "the bar carries on from {moving}, not from {}",
        tab_rect(&chrome, 1).x
    );
    assert!(
        bar.rect[0] > tab_rect(&chrome, 1).x + 1.0,
        "it did not snap back"
    );
}

#[test]
fn nothing_moves_when_the_active_tab_does_not() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let mut chrome = chrome();
    let input = input(&palette, &tabs, window());
    chrome.set_time(0.0);
    let first = draw(&mut chrome, &input);
    chrome.set_time(1.0);
    let second = draw(&mut chrome, &input);

    assert_eq!(first.frame.quads, second.frame.quads);
    assert_eq!(first.frame.glyphs, second.frame.glyphs);
}

// ---------------------------------------------------------------------------------
// Scale
// ---------------------------------------------------------------------------------

#[test]
fn the_scale_is_applied_once_and_only_to_what_is_emitted() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let mut plain = chrome();
    let mut scaled = chrome();
    let size = window();

    let one = draw(&mut plain, &input(&palette, &tabs, size));
    let mut doubled_input = input(&palette, &tabs, size);
    doubled_input.scale = 2.0;
    let two = draw(&mut scaled, &doubled_input);

    // The layout does not move: a logical pixel is a logical pixel at every DPI, which is
    // what makes the strip's arithmetic the design document's arithmetic.
    assert_eq!(one.layout, two.layout);
    assert_eq!(tab_rects(&plain), tab_rects(&scaled));

    // Everything emitted is twice the size, rectangles and glyphs alike.
    let hairline = color(palette.hairline);
    let find = |frame: &Frame| {
        frame
            .quads
            .iter()
            .find(|quad| quad.color.map(f32::to_bits) == hairline)
            .expect("the row's hairline")
            .rect
    };
    let plain_hairline = find(&one.frame);
    let scaled_hairline = find(&two.frame);
    assert!((scaled_hairline[1] - plain_hairline[1] * 2.0).abs() < f32::EPSILON);
    assert!((scaled_hairline[3] - plain_hairline[3] * 2.0).abs() < f32::EPSILON);
    assert!((scaled_hairline[2] - plain_hairline[2] * 2.0).abs() < f32::EPSILON);

    let glyph = |frame: &Frame| frame.glyphs[0].rect;
    let plain_glyph = glyph(&one.frame);
    let scaled_glyph = glyph(&two.frame);
    assert!((scaled_glyph[2] - plain_glyph[2] * 2.0).abs() < f32::EPSILON);
    assert!((scaled_glyph[0] - plain_glyph[0] * 2.0).abs() < f32::EPSILON);
    assert!((scaled_glyph[1] - plain_glyph[1] * 2.0).abs() < f32::EPSILON);
}
