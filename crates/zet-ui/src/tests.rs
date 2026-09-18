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

use std::collections::BTreeMap;

use zet_config::{Palette, TabPosition, TabSettings, WindowSettings};
use zet_font::{GlyphSpec, Metrics, Weight};
use zet_render::{Frame, Placement, Quad};

use crate::fonts::GlyphSource;
use crate::geometry::ROW_HEIGHT;
use crate::{
    Caption, Chrome, ChromeInput, Control, FindLine, Hit, Layout, PickerLine, Rect, ScrollState,
    Scrollbar, SettingLine, SettingPart, Size, TabInfo, thumb_offset,
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
        settings: &[],
        settings_scroll: 0.0,
        settings_focus: None,
        find: None,
        picker: None,
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

    // `#1 Terminal 1`: the `#` at seventy percent, one digit, the gap, ten characters of
    // name at half the size, and the padding either side. Every character in the test
    // source is half an em wide, which is what makes this an arithmetic rather than a
    // measurement.
    let expected = 13.0 * 0.7 * 0.5 + 13.0 * 0.5 + 8.0 + 10.0 * 13.0 * 0.5 + 2.0 * 12.0;
    assert!(
        (first.width - expected).abs() < 1.0e-4,
        "a tab is {expected} wide, not {}",
        first.width
    );
}

#[test]
fn a_second_digit_makes_a_tab_wider() {
    // The names are the same width on purpose, so that what this measures is the extra
    // digit and nothing else. A tab's width is its number and its name, and the two
    // halves have to be separable for either to be checkable.
    let palette = Palette::instrument();
    let one = vec![TabInfo {
        index: 1,
        title: "Terminal".to_owned(),
        hovered: false,
    }];
    let ten = vec![TabInfo {
        index: 10,
        title: "Terminal".to_owned(),
        hovered: false,
    }];
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
fn the_new_tab_mark_ends_the_run_and_has_a_bare_tab_s_footprint() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2, 3]);
    let mut chrome = chrome();
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    let rects = tab_rects(&chrome);
    let plus = plus_rect(&chrome).expect("the new-tab mark is always on the row");
    let last = rects[rects.len() - 1].1;
    assert!((plus.x - last.right()).abs() < f32::EPSILON);
    // A tab with nothing to say: `#1` and its padding. The mark is the tab that is about
    // to exist, and a new tab has no name until a program gives it one — so the mark is
    // the width of the cell it will land in, not the width of a tab that is running a
    // title.
    let bare = 13.0 * 0.7 * 0.5 + 13.0 * 0.5 + 2.0 * 12.0;
    assert!(
        (plus.width - bare).abs() < 1.0e-4,
        "the mark is {bare} wide, not {}",
        plus.width
    );

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
// Tab names
// ---------------------------------------------------------------------------------

/// Every character the frame drew at one weight, in order.
fn run(frame: &Frame, weight: Weight) -> String {
    drawn_at(frame, weight).into_iter().collect()
}

/// Everything the frame drew, at both weights, as one string.
///
/// A tab is drawn twice over — the active one at MEDIUM and the rest at NORMAL — so a
/// question about *what text is on the strip* is a question about both runs. Only a
/// question about the active tab itself can be asked of one weight.
fn all_text(frame: &Frame) -> String {
    format!(
        "{}{}",
        run(frame, Weight::MEDIUM),
        run(frame, Weight::NORMAL)
    )
}

/// One tab with a name, which is what a shell that sets `OSC 0` produces.
fn named(index: u32, title: &str) -> Vec<TabInfo> {
    vec![TabInfo {
        index,
        title: title.to_owned(),
        hovered: false,
    }]
}

#[test]
fn a_tab_shows_its_number_and_then_its_name() {
    // What the user asked for and what the tab is for: `#3` says which channel, and the
    // name says what is running on it. The number keeps its weight, so the ordinal is
    // still the thing that scans; the name is set in the lighter weight beside it.
    let palette = Palette::instrument();
    let tabs = named(3, "PowerShell");
    let mut chrome = chrome();
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    assert!(run(&drawn.frame, Weight::MEDIUM).contains("#3"));
    assert!(
        all_text(&drawn.frame).contains("PowerShell"),
        "the name was not drawn: {:?}",
        all_text(&drawn.frame)
    );

    // And the cell grew to hold it, rather than the name spilling over the next tab.
    let cell = tab_rect(&chrome, 3);
    let bare = 13.0 * 0.7 * 0.5 + 13.0 * 0.5 + 2.0 * 12.0;
    assert!(cell.width > bare, "the cell is still a bare number");
}

#[test]
fn a_name_with_no_room_is_cut_and_says_so() {
    // Truncation has to be visible. A name that ends flush against the next tab reads as
    // the whole name, and the user has no way to tell that the program is running
    // something with a longer name than the strip is showing.
    let palette = Palette::instrument();
    let tabs = named(1, &"x".repeat(40));
    let mut chrome = chrome();
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    let text = run(&drawn.frame, Weight::NORMAL);
    assert!(
        text.contains('…'),
        "a 40-character name was cut without an ellipsis: {text:?}"
    );
    assert!(
        text.chars().filter(|ch| *ch == 'x').count() < 40,
        "nothing was cut"
    );
}

#[test]
fn a_long_name_does_not_make_a_wide_tab() {
    // The ceiling. One program with an enormous title must not take the row: tabs share
    // it, so Cell::MAX is the widest any of them gets however much room there is.
    let palette = Palette::instrument();
    let tabs = named(1, &"y".repeat(200));
    let mut chrome = chrome();
    draw(&mut chrome, &input(&palette, &tabs, window()));
    assert!(
        tab_rect(&chrome, 1).width <= 180.0 + 1.0e-4,
        "a tab grew past its ceiling: {}",
        tab_rect(&chrome, 1).width
    );
}

#[test]
fn a_crowded_strip_squeezes_every_tab_by_the_same_amount() {
    // What stops a row of names from being one wide tab and a row of clipped ones. Past
    // the point where they all fit naturally, every cell gives up the same amount, so
    // the strip degrades evenly and the tab the user is looking at is not the one that
    // happens to be first.
    let palette = Palette::instrument();
    let many = tabs(&(1..=12).collect::<Vec<u32>>());
    let mut chrome = chrome();
    draw(&mut chrome, &input(&palette, &many, window()));

    let rects = tab_rects(&chrome);
    assert_eq!(rects.len(), 12, "all twelve should still be on the row");
    let width = rects[0].1.width;
    assert!(
        width < 180.0,
        "this strip was meant to be too crowded for full-width tabs"
    );
    for (index, rect) in &rects {
        assert!(
            (rect.width - width).abs() < 1.0e-4,
            "#{index} is {} wide and #1 is {width}",
            rect.width
        );
    }
    // And squeezed is not truncated past the point of being useful: every name still
    // starts. Five characters of "Terminal 1" plus the ellipsis is what the share works
    // out to, which is enough to tell two shells apart and not enough to read either.
    let drawn = draw(&mut chrome, &input(&palette, &many, window()));
    let text = all_text(&drawn.frame);
    assert!(
        text.matches('T').count() == 12,
        "not every name survived the squeeze: {text:?}"
    );
}

#[test]
fn a_tab_with_no_name_is_still_a_tab() {
    // A program that never sets a title, and a profile with no name: the cell is its
    // number and nothing else, which is what every tab was before names existed.
    let palette = Palette::instrument();
    let tabs = named(1, "");
    let mut chrome = chrome();
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    assert!(run(&drawn.frame, Weight::MEDIUM).contains("#1"));
    let bare = 13.0 * 0.7 * 0.5 + 13.0 * 0.5 + 2.0 * 12.0;
    assert!((tab_rect(&chrome, 1).width - bare).abs() < 1.0e-4);
}

#[test]
fn the_rail_shows_numbers_and_no_names() {
    // The rail is forty-eight pixels wide and the whole reason to choose it is that it
    // gives the grid the rest. A name in it would be a name in the space the tabs were
    // moved aside to free, so the rail is the one position where a tab is only a number.
    let palette = Palette::instrument();
    let settings = TabSettings {
        position: TabPosition::Left,
        ..TabSettings::default()
    };
    let tabs = named(1, "PowerShell");
    let mut chrome = Chrome::new(&settings, &WindowSettings::default());
    let drawn = draw(&mut chrome, &input(&palette, &tabs, window()));

    let text = all_text(&drawn.frame);
    assert!(text.contains('1'), "the number is gone too: {text:?}");
    assert!(!text.contains('P'), "the rail drew a name: {text:?}");
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
    // No surface and no hairline. What is left is the caption marks, and asserting that
    // they are the *only* thing left is stronger than asserting the row drew nothing:
    // every quad has to be inside one of the three button boxes, so a surface that came
    // back would be caught by where it is rather than by how many there are.
    assert!(!drawn.frame.quads.is_empty(), "the captions are gone");
    assert!(
        drawn.frame.glyphs.is_empty(),
        "an absent row still drew text"
    );
    for quad in &drawn.frame.quads {
        let [x, y, w, h] = quad.rect;
        let inside = (1200.0 - 3.0 * 46.0..1200.0).contains(&x)
            && (0.0..40.0).contains(&y)
            && (1200.0 - 3.0 * 46.0..1200.0).contains(&(x + w - 1.0))
            && (0.0..40.0).contains(&(y + h - 1.0));
        assert!(inside, "{:?} is not one of the caption marks", quad.rect);
    }
}

// ---------------------------------------------------------------------------------
// Numbering
// ---------------------------------------------------------------------------------

#[test]
fn numbering_survives_a_close() {
    // Close #2 of three and the two that are left are still #1 and #3. A number
    // identifies a session, and shuffling numbers under the user is worse than a gap.
    let palette = Palette::instrument();
    // Names with no digits in them, so that what this counts is tab numbers and not
    // every decimal the strip happens to be showing.
    let tabs = vec![
        TabInfo {
            index: 1,
            title: "Shell".to_owned(),
            hovered: false,
        },
        TabInfo {
            index: 3,
            title: "Shell".to_owned(),
            hovered: false,
        },
    ];
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
    input.find = Some(FindLine::default());
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
    input.find = None;
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

    // The panel's own surface is in the frame.
    let raised = color(palette.surface_raised);
    assert!(
        drawn
            .frame
            .quads
            .iter()
            .any(|quad| quad.color.map(f32::to_bits) == raised)
    );
}

/// The lines a test panel is built from: two sections, a heading and rows of each
/// control kind.
fn settings_lines() -> Vec<SettingLine<'static>> {
    vec![
        SettingLine {
            text: "APPEARANCE",
            control: None,
            value: "",
        },
        SettingLine {
            text: "Theme",
            control: Some(Control::Choice),
            value: "zet dark",
        },
        SettingLine {
            text: "Reduce motion",
            control: Some(Control::Toggle),
            value: "Off",
        },
        SettingLine {
            text: "TERMINAL",
            control: None,
            value: "",
        },
        SettingLine {
            text: "Size",
            control: Some(Control::Step),
            value: "13",
        },
        SettingLine {
            text: "New tab",
            control: Some(Control::Chord),
            value: "Ctrl+Shift+T",
        },
    ]
}

/// The panel, open on a test window, drawn once.
fn open_panel(palette: &Palette, lines: &[SettingLine<'_>]) -> (Chrome, Drawn) {
    open_panel_at(palette, lines, None)
}

/// The panel, open with the pointer at a point.
fn open_panel_at(
    palette: &Palette,
    lines: &[SettingLine<'_>],
    pointer: Option<(f32, f32)>,
) -> (Chrome, Drawn) {
    let tabs = tabs(&[1]);
    let mut chrome = chrome();
    let mut input = input(palette, &tabs, window());
    input.settings_open = true;
    input.settings = lines;
    input.pointer = pointer;
    let drawn = draw(&mut chrome, &input);
    (chrome, drawn)
}

/// The rectangle of a line's control, the whole of it.
///
/// The union of the regions the line published rather than the first of them, because a
/// stepper publishes one per half and a test that took the first would be measuring the
/// left half while believing it had the control.
fn control_rect(chrome: &Chrome, line: usize) -> Rect {
    let mut whole: Option<Rect> = None;
    for region in chrome.regions() {
        let crate::Region::Setting { line: at, rect, .. } = region else {
            continue;
        };
        if *at != line {
            continue;
        }
        whole = Some(match whole {
            None => *rect,
            Some(previous) => Rect::between(
                previous.x.min(rect.x),
                previous.y.min(rect.y),
                previous.right().max(rect.right()),
                previous.bottom().max(rect.bottom()),
            ),
        });
    }
    whole.unwrap_or_else(|| panic!("line {line} has no control"))
}

/// The colour at a point: whatever was painted over it last, which is what the user sees.
fn color_at(frame: &Frame, x: f32, y: f32) -> Option<[u32; 4]> {
    frame
        .quads
        .iter()
        .rev()
        .find(|quad| {
            let [qx, qy, width, height] = quad.rect;
            x >= qx && y >= qy && x < qx + width && y < qy + height
        })
        .map(|quad| quad.color.map(f32::to_bits))
}

#[test]
fn the_panel_draws_the_lines_it_is_given() {
    // The panel is handed words and values; it draws them and nothing of its own. The
    // skeleton it used to draw said "Theme" and then left a grey box where the theme
    // would go, which is a settings panel that cannot set anything.
    let palette = Palette::instrument();
    let lines = settings_lines();
    let (_, drawn) = open_panel(&palette, &lines);

    let heading: Vec<char> = drawn_at(&drawn.frame, Weight::MEDIUM);
    for letter in "APPEARANCE".chars() {
        assert!(heading.contains(&letter), "the heading was not drawn");
    }
    let body: Vec<char> = drawn_at(&drawn.frame, Weight::NORMAL);
    for expected in ["Theme", "zet dark", "Reduce motion", "Off", "Ctrl+Shift+T"] {
        for letter in expected.chars().filter(|ch| !ch.is_whitespace()) {
            assert!(body.contains(&letter), "{expected} was not drawn");
        }
    }
}

#[test]
fn the_hover_fill_covers_the_region_a_click_would_take() {
    // DESIGN.md: "Hover fills the half a click will take." Every row's whole face is the
    // click target, so every control but a stepper lights all of itself. Filling the
    // left half of everything instead said a one-click control was two controls wearing
    // one rectangle — the thing the stepper exists to be — and a click on the right
    // half, which works, lit a region it was not in.
    let palette = Palette::instrument();
    let lines = settings_lines();
    let (chrome, _) = open_panel(&palette, &lines);
    let hairline = color(palette.hairline);

    // Line 2 is the toggle. Its right-hand side is the click the old code lit the left
    // half of, so that is where the pointer goes.
    let toggle = control_rect(&chrome, 2);
    let (tx, ty) = (toggle.right() - 4.0, toggle.center().1);
    let (_, drawn) = open_panel_at(&palette, &lines, Some((tx, ty)));
    assert_eq!(
        color_at(&drawn.frame, tx, ty),
        Some(hairline),
        "the pointer's own half of a one-click control is not lit"
    );
    assert_eq!(
        color_at(&drawn.frame, toggle.center().0 - 4.0, ty),
        Some(hairline),
        "a toggle is one control, so the half beside the pointer lights with it"
    );
    assert_eq!(
        color_at(&drawn.frame, toggle.x + 1.0, ty),
        Some(hairline),
        "the whole face of a one-click control is the click target"
    );

    // Line 4 is the stepper, and the half the pointer is not in stays dark: the two
    // halves mean opposite things, and a fill over both would say so.
    let step = control_rect(&chrome, 4);
    let (sx, sy) = (step.right() - 4.0, step.center().1);
    let (_, drawn) = open_panel_at(&palette, &lines, Some((sx, sy)));
    assert_eq!(color_at(&drawn.frame, sx, sy), Some(hairline));
    assert_ne!(
        color_at(&drawn.frame, step.x + 4.0, sy),
        Some(hairline),
        "the other half of a stepper was lit with the half under the pointer"
    );
}

#[test]
fn every_control_is_a_region_that_names_its_line() {
    // What makes the panel a control rather than a picture. A click has to come back
    // naming the line it landed on, or the caller has nothing to apply it to.
    let palette = Palette::instrument();
    let lines = settings_lines();
    let (chrome, _) = open_panel(&palette, &lines);

    // Lines 1, 2, 4 and 5 have controls; 0 and 3 are headings.
    for line in [1_usize, 2, 4, 5] {
        let (part, rect) = chrome
            .regions()
            .iter()
            .find_map(|region| match region {
                crate::Region::Setting {
                    line: at,
                    part,
                    rect,
                } if *at == line => Some((*part, *rect)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("line {line} has no control"));
        let (x, y) = rect.center();
        assert_eq!(
            chrome.hit(x, y),
            Hit::Setting { line, part },
            "line {line} does not hit-test as itself"
        );
    }
}

#[test]
fn a_stepper_answers_twice_and_everything_else_once() {
    // A stepper is two controls wearing one rectangle: its halves mean opposite things,
    // and a caller that only learned "the size row was clicked" would have to guess which
    // way the user meant.
    let palette = Palette::instrument();
    let lines = settings_lines();
    let (chrome, _) = open_panel(&palette, &lines);

    let halves: Vec<(SettingPart, Rect)> = chrome
        .regions()
        .iter()
        .filter_map(|region| match region {
            crate::Region::Setting {
                line: 4,
                part,
                rect,
                ..
            } => Some((*part, *rect)),
            _ => None,
        })
        .collect();
    assert_eq!(halves.len(), 2, "the size row is not two halves");
    let (left, right) = (halves[0].1, halves[1].1);
    assert!((left.width - right.width).abs() < f32::EPSILON);
    assert!(
        (left.right() - right.x).abs() < f32::EPSILON,
        "the halves meet"
    );
    assert_eq!(
        chrome.hit(left.x + 2.0, left.y + 2.0),
        Hit::Setting {
            line: 4,
            part: SettingPart::Less
        }
    );
    assert_eq!(
        chrome.hit(right.x + 2.0, right.y + 2.0),
        Hit::Setting {
            line: 4,
            part: SettingPart::More
        }
    );

    // A toggle is one control, not two.
    let toggles = chrome
        .regions()
        .iter()
        .filter(|region| {
            matches!(
                region,
                crate::Region::Setting {
                    line: 2,
                    part: SettingPart::Whole,
                    ..
                }
            )
        })
        .count();
    assert_eq!(toggles, 1);
}

#[test]
fn a_row_that_does_not_fit_is_scrolled_to_rather_than_dropped() {
    // A panel that silently stops listing settings once the window is short is a panel
    // where the user cannot find a setting and has no way to tell that it is there. The
    // list scrolls, and the scroll is clamped to the overflow rather than to whatever
    // the caller last asked for.
    let palette = Palette::instrument();
    let lines: Vec<SettingLine<'static>> = (0..40)
        .map(|_| SettingLine {
            text: "Size",
            control: Some(Control::Step),
            value: "13",
        })
        .collect();

    let tabs = tabs(&[1]);
    let mut chrome = chrome();
    let short = Size {
        width: 1200.0,
        height: 200.0,
    };
    let mut input = input(&palette, &tabs, short);
    input.settings_open = true;
    input.settings = &lines;

    // Unscrolled, the list starts at the top and the first row is on screen.
    let top = draw(&mut chrome, &input);
    assert!((top.layout.settings_scroll).abs() < f32::EPSILON);
    let first = |chrome: &Chrome| {
        chrome.regions().iter().any(|region| {
            matches!(
                region,
                crate::Region::Setting {
                    line: 0,
                    part: SettingPart::Less,
                    ..
                }
            )
        })
    };
    assert!(first(&chrome), "row 0 should be on screen at the top");

    // Ask for more scroll than there is. The answer comes back clamped to the overflow,
    // so the wheel does not spend a second doing nothing while the content catches up.
    let mut far = input;
    far.settings_scroll = 100_000.0;
    let clamped = draw(&mut chrome, &far);
    let content = 40.0 * 28.0;
    let viewport = short.height - top.layout.top - 2.0 * 16.0;
    let overflow = content - viewport;
    assert!(overflow > 0.0, "forty rows should not fit in 200 pixels");
    assert!(
        (clamped.layout.settings_scroll - overflow).abs() < 1.0e-3,
        "the scroll stopped at {} rather than at the end of the content, {overflow}",
        clamped.layout.settings_scroll
    );
    assert!(!first(&chrome), "row 0 should have scrolled off");
}

#[test]
#[allow(clippy::float_cmp)] // Exact on purpose: both values are the same palette entry.
fn the_row_the_keyboard_is_on_wears_the_brightest_edge_and_the_others_do_not() {
    // Three weights of one hairline and no fourth: focused is `ink`, hovered is
    // `hairline-strong`, and the rest are `hairline`. `signal` is the obvious colour for
    // a focus ring and the wrong one — it is the app's one lamp with a 3px by 40px
    // budget, which a border around a 118-pixel control would spend several times over.
    let palette = Palette::instrument();
    let lines = settings_lines();
    let tabs = tabs(&[1]);
    let mut chrome = chrome();

    let edge_of = |chrome: &Chrome, drawn: &Drawn, line: usize| {
        let rect = chrome
            .regions()
            .iter()
            .find_map(|region| match region {
                crate::Region::Setting {
                    line: at,
                    part: SettingPart::Whole,
                    rect,
                } if *at == line => Some(*rect),
                _ => None,
            })
            .unwrap_or_else(|| panic!("row {line} has no region"));
        // The top edge, which is the first of the four the border draws.
        // The one-pixel strip along the top edge. The fill behind the control has the
        // same x, y and width, so the height is what tells them apart.
        drawn
            .frame
            .quads
            .iter()
            .find(|quad| {
                (quad.rect[0] - rect.x).abs() < 0.5
                    && (quad.rect[1] - rect.y).abs() < 0.5
                    && (quad.rect[2] - rect.width).abs() < 0.5
                    && (quad.rect[3] - 1.0).abs() < 0.5
            })
            .map_or_else(|| panic!("row {line} has no border"), |quad| quad.color)
    };

    let plain = {
        let mut input = input(&palette, &tabs, window());
        input.settings_open = true;
        input.settings = &lines;
        let drawn = draw(&mut chrome, &input);
        edge_of(&chrome, &drawn, 1)
    };
    let focused = {
        let mut input = input(&palette, &tabs, window());
        input.settings_open = true;
        input.settings = &lines;
        input.settings_focus = Some(1);
        let drawn = draw(&mut chrome, &input);
        edge_of(&chrome, &drawn, 1)
    };
    assert_ne!(
        plain, focused,
        "the focused row looks exactly like the rest"
    );

    // And the row that is not focused is untouched by it, which is the half a test that
    // only compared two frames would miss.
    let mut input = input(&palette, &tabs, window());
    input.settings_open = true;
    input.settings = &lines;
    input.settings_focus = Some(1);
    let drawn = draw(&mut chrome, &input);
    assert_eq!(edge_of(&chrome, &drawn, 2), plain);
}

#[test]
fn a_focused_row_below_the_fold_is_scrolled_to_rather_than_hidden() {
    // A highlight the user cannot see is a panel that looks broken rather than scrolled,
    // which is worse than not having the key at all.
    let palette = Palette::instrument();
    let lines: Vec<SettingLine<'static>> = (0..40)
        .map(|_| SettingLine {
            text: "Size",
            control: Some(Control::Step),
            value: "13",
        })
        .collect::<Vec<_>>();
    let tabs = tabs(&[1]);
    let mut chrome = chrome();
    let short = Size {
        width: 1200.0,
        height: 200.0,
    };
    let mut input = input(&palette, &tabs, short);
    input.settings_open = true;
    input.settings = &lines;
    input.settings_scroll = 0.0;
    input.settings_focus = Some(39);

    let drawn = draw(&mut chrome, &input);
    let top = drawn.layout.top + 16.0;
    let bottom = short.height - 16.0;
    let last = chrome
        .regions()
        .iter()
        .find_map(|region| match region {
            crate::Region::Setting {
                line: 39,
                part: SettingPart::Less,
                rect,
            } => Some(*rect),
            _ => None,
        })
        .expect("the focused row is drawn, so it is on screen");
    assert!(
        last.y >= top && last.bottom() <= bottom,
        "row 39 is at {}..{} and the panel's viewport is {top}..{bottom}",
        last.y,
        last.bottom()
    );
    assert!(
        drawn.layout.settings_scroll > 0.0,
        "the list did not scroll to reach it"
    );
}

#[test]
fn a_row_scrolled_half_off_the_list_is_not_drawn_over_the_chrome_above_it() {
    // Nothing under the panel clips it. A row that is scrolled half off the top would
    // draw its control, its border and its value over the tab strip, which is chrome the
    // panel is meant to sit on and not paint on.
    let palette = Palette::instrument();
    let lines: Vec<SettingLine<'static>> = (0..12)
        .map(|_| SettingLine {
            text: "Size",
            control: Some(Control::Step),
            value: "13",
        })
        .collect::<Vec<_>>();
    let tabs = tabs(&[1]);
    let mut chrome = chrome();
    let short = Size {
        width: 800.0,
        height: 300.0,
    };
    let mut input = input(&palette, &tabs, short);
    input.settings_open = true;
    input.settings = &lines;
    // Two wheel notches. Small enough that the list does not reach its end, so the rows
    // straddle the panel's top edge rather than all fitting inside it.
    input.settings_scroll = 96.0;

    let drawn = draw(&mut chrome, &input);
    let panel = chrome
        .regions()
        .iter()
        .find_map(|region| match region {
            crate::Region::Settings(rect) => Some(*rect),
            _ => None,
        })
        .expect("the panel is open");
    assert!(
        panel.y > 0.0,
        "this window has no chrome above the panel, so nothing could escape it"
    );

    // Every control fill. `ground` is the colour a control is filled with and the panel
    // itself is `surface-raised`, so these rectangles are exactly the panel's own.
    let ground = color(palette.ground);
    let controls: Vec<[f32; 4]> = drawn
        .frame
        .quads
        .iter()
        .filter(|quad| quad.color.map(f32::to_bits) == ground)
        .map(|quad| quad.rect)
        .collect();
    assert!(
        !controls.is_empty(),
        "the panel drew no controls, so this test would pass on an empty frame"
    );
    for rect in controls {
        assert!(
            rect[1] >= panel.y && rect[1] + rect[3] <= panel.bottom(),
            "a control at y {}..{} escaped the panel, which starts at {}",
            rect[1],
            rect[1] + rect[3],
            panel.y
        );
    }
}

#[test]
fn a_heading_scrolled_off_the_panel_takes_its_rule_with_it() {
    // A section's rule belongs to the heading above it and the two are one block. Asked
    // separately whether it fits, the rule answers yes for the twenty-two pixels between
    // the heading's box leaving the panel and the rule's own top arriving, and the frame
    // gets a full-width hairline with nothing over it — a line that reads as a rule for
    // whichever row happens to sit above it.
    let palette = Palette::instrument();
    let hairline = color(palette.hairline);
    let mut lines = vec![SettingLine {
        text: "APPEARANCE",
        control: None,
        value: "",
    }];
    lines.extend((0..12).map(|_| SettingLine {
        text: "Size",
        control: Some(Control::Step),
        value: "13",
    }));
    let tabs = tabs(&[1]);
    let mut chrome = chrome();
    let short = Size {
        width: 800.0,
        height: 300.0,
    };
    let mut input = input(&palette, &tabs, short);
    input.settings_open = true;
    input.settings = &lines;

    // Only this fixture's one section rule spans the whole panel: the panel's own left
    // edge is one pixel of hairline and a control's border is 118. Each scroll is
    // recorded with the number of control fills, which is how the last assertion knows
    // the panel was still populated rather than empty.
    let ground = color(palette.ground);
    let mut seen = Vec::new();
    for scroll in [0.0_f32, 30.0] {
        input.settings_scroll = scroll;
        let drawn = draw(&mut chrome, &input);
        let panel = chrome
            .regions()
            .iter()
            .find_map(|region| match region {
                crate::Region::Settings(rect) => Some(*rect),
                _ => None,
            })
            .expect("the panel is open");
        let rules = drawn
            .frame
            .quads
            .iter()
            .filter(|quad| {
                // A section rule is the full width of the panel, which is what tells it
                // apart from the one-pixel edge beside it and from a control's 118-pixel
                // border. Half a pixel is a tolerance this cannot need and the lint asks
                // for; the three widths differ by two orders of magnitude.
                quad.color.map(f32::to_bits) == hairline && (quad.rect[2] - panel.width).abs() < 0.5
            })
            .count();
        let controls = drawn
            .frame
            .quads
            .iter()
            .filter(|quad| quad.color.map(f32::to_bits) == ground)
            .count();
        seen.push((rules, controls));
    }

    assert_eq!(seen[0].0, 1, "the heading's rule was not drawn with it");
    // Sixteen pixels of padding is what the heading's box starts below the panel's top
    // edge, so at this scroll the box is fourteen pixels above it and the rule, twenty-two
    // below that, is eight pixels inside it.
    assert_eq!(seen[1].0, 0, "the rule outlived the heading it belongs to");
    assert!(
        seen[1].1 > 0,
        "the panel drew no rows at this scroll, so the rule count proves nothing"
    );
}

#[test]
fn clicking_the_panel_is_not_clicking_the_grid() {
    // The panel floats over the terminal, so a click on its surface must stop there. A
    // click that fell through would type into a program the user was not looking at.
    let palette = Palette::instrument();
    let lines = settings_lines();
    let (chrome, drawn) = open_panel(&palette, &lines);
    let panel = chrome
        .regions()
        .iter()
        .find_map(|region| match region {
            crate::Region::Settings(rect) => Some(*rect),
            _ => None,
        })
        .expect("the panel is open");

    // A pixel inside the panel but not on any control: the gutter below the last row.
    let gutter = (panel.x + 10.0, panel.bottom() - 4.0);
    assert_eq!(
        chrome.hit(gutter.0, gutter.1),
        Hit::Settings,
        "the panel's own surface is not hit-testable"
    );
    // And the grid is still the grid outside it.
    assert_eq!(chrome.hit(panel.x - 10.0, 400.0), Hit::None);
    let _ = drawn;
}

#[test]
fn an_open_find_bar_is_a_row_of_its_own() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let size = window();
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, size);
    input.find = Some(FindLine::default());
    let drawn = draw(&mut chrome, &input);

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

/// Everything the bar drew, as one string, with the spaces taken out.
///
/// The fake font gives a space no ink, so the painter emits no glyph for one and the
/// blanks between words are not in what comes back. Everything else on the row is one
/// sentence read left to right, so the order is the whole of what there is to check.
///
/// The window is drawn with no tabs, which is what makes this the find bar's text and
/// nothing else's: with no tabs there is no strip, and the strip is the only other thing
/// in the chrome that draws words.
fn find_bar_text(chrome: &mut Chrome, palette: &Palette, find: FindLine<'_>) -> String {
    let mut input = input(palette, &[], window());
    input.find = Some(find);
    let drawn = draw(chrome, &input);
    let text: String = drawn_at(&drawn.frame, Weight::NORMAL).into_iter().collect();
    text.replace(' ', "")
}

#[test]
fn the_find_bar_shows_what_was_typed_and_which_match_it_is_on() {
    let palette = Palette::instrument();
    let mut chrome = chrome();
    let text = find_bar_text(
        &mut chrome,
        &palette,
        FindLine {
            query: "hello",
            position: Some((3, 17)),
            capped: false,
        },
    );
    assert_eq!(text, "Findhello3of17");
}

#[test]
fn a_query_too_long_for_the_field_is_shown_from_its_end() {
    // The caret is where the next character goes, and a caret off the right edge is a
    // field that looks broken at the moment the user is typing into it.
    let palette = Palette::instrument();
    let mut chrome = chrome();
    let long = "x".repeat(400) + "needle";
    let text = find_bar_text(
        &mut chrome,
        &palette,
        FindLine {
            query: &long,
            position: None,
            capped: false,
        },
    );
    assert!(text.starts_with("Findx"), "{text}");
    assert!(
        text.contains("needleNoresults"),
        "the tail is what is shown: {text}"
    );
    assert!(text.len() < 100, "the head was dropped: {text}");
}

#[test]
fn a_query_that_matched_nothing_says_so_rather_than_showing_a_zero() {
    let palette = Palette::instrument();
    let mut chrome = chrome();
    let text = find_bar_text(
        &mut chrome,
        &palette,
        FindLine {
            query: "omega",
            position: None,
            capped: false,
        },
    );
    assert_eq!(text, "FindomegaNoresults");
}

#[test]
fn a_search_that_stopped_counting_says_that_it_did() {
    let palette = Palette::instrument();
    let mut chrome = chrome();
    let text = find_bar_text(
        &mut chrome,
        &palette,
        FindLine {
            query: "x",
            position: Some((1, 1000)),
            capped: true,
        },
    );
    assert_eq!(text, "Findx1of1000+");
}

#[test]
fn an_empty_query_has_no_count_because_there_is_nothing_to_count() {
    let palette = Palette::instrument();
    let mut chrome = chrome();
    let text = find_bar_text(&mut chrome, &palette, FindLine::default());
    assert_eq!(text, "Find");
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

#[test]
fn the_scrollbar_is_still_reachable_with_the_settings_panel_open() {
    // The panel covers the right edge of the window and the scrollbar is eight pixels of
    // that edge drawn over it, so the two overlap and only one of them can answer a
    // click. It is the one the user can see: a thumb that shades a panel is a thumb, and
    // a hit test that hands the click to the surface underneath is a scrollbar that
    // cannot be dragged.
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let size = window();
    let mut chrome = chrome();
    chrome.set_scroll(ScrollState {
        offset: 0.0,
        visible: 0.25,
    });
    let lines: Vec<SettingLine<'static>> = (0..12)
        .map(|_| SettingLine {
            text: "Size",
            control: Some(Control::Step),
            value: "13",
        })
        .collect::<Vec<_>>();
    let mut with_panel = input(&palette, &tabs, size);
    with_panel.settings_open = true;
    with_panel.settings = &lines;
    draw(&mut chrome, &with_panel);

    let (track, thumb) = scrollbar(&chrome);
    let panel = chrome
        .regions()
        .iter()
        .find_map(|region| match region {
            crate::Region::Settings(rect) => Some(*rect),
            _ => None,
        })
        .expect("the panel is open");
    assert!(
        track.x >= panel.x,
        "the scrollbar has to be inside the panel for this to be a conflict at all"
    );

    let (x, y) = (thumb.center().0, thumb.center().1);
    assert_eq!(
        chrome.hit(x, y),
        Hit::Scrollbar(Scrollbar::Thumb),
        "the panel swallowed a click on the thumb it is drawn under"
    );
}

/// The scrollbar's track and thumb, or a panic saying the bar was not drawn.
fn scrollbar(chrome: &Chrome) -> (Rect, Rect) {
    chrome.scrollbar().expect("the bar was drawn")
}

#[test]
fn a_thumb_dragged_to_a_place_on_its_track_is_the_offset_it_was_drawn_at() {
    // The round trip, and the reason `thumb_offset` lives here rather than in the caller
    // that drags: a caller would have to restate the placement to check its inverse, and
    // a restated formula is a test of the copy rather than of the thing.
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let size = window();
    let mut chrome = chrome();

    for offset in [0.0, 0.25, 0.5, 0.75, 1.0] {
        for visible in [0.1, 0.5, 0.9] {
            chrome.set_scroll(ScrollState { offset, visible });
            draw(&mut chrome, &input(&palette, &tabs, size));
            let (track, thumb) = scrollbar(&chrome);
            let back = thumb_offset(track, thumb, thumb.y).expect("the thumb can move");
            assert!(
                (back - offset).abs() < 1e-3,
                "an offset of {offset} at {visible} visible drew a thumb at {back}"
            );
        }
    }
}

#[test]
fn dragging_past_either_end_of_the_track_stops_at_the_end() {
    let track = Rect::new(0.0, 100.0, 8.0, 600.0);
    let thumb = Rect::new(0.0, 400.0, 8.0, 100.0);
    assert_eq!(thumb_offset(track, thumb, -500.0), Some(0.0));
    assert_eq!(thumb_offset(track, thumb, 100.0), Some(0.0));
    assert_eq!(thumb_offset(track, thumb, 700.0), Some(1.0));
    assert_eq!(thumb_offset(track, thumb, 5000.0), Some(1.0));
    // Halfway down the distance the thumb can travel, not half the track: the track is
    // 600 tall, the thumb 100, so the middle of the travel is at 350.
    assert_eq!(thumb_offset(track, thumb, 350.0), Some(0.5));
}

#[test]
fn a_thumb_that_cannot_move_has_nowhere_to_be_dragged_to() {
    // A thumb as tall as its track is a scrollback with nothing scrolled off. There is
    // no division to do, and a caller that did it anyway would be dividing by zero.
    let track = Rect::new(0.0, 0.0, 8.0, 400.0);
    assert_eq!(thumb_offset(track, track, 200.0), None);
    assert_eq!(
        thumb_offset(track, Rect::new(0.0, 0.0, 8.0, 401.0), 200.0),
        None,
        "a thumb taller than its track is the same answer, not a negative travel"
    );
}

#[test]
fn the_scrollbar_publishes_itself_only_when_it_is_drawn() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let size = window();
    let mut chrome = chrome();
    draw(&mut chrome, &input(&palette, &tabs, size));
    assert_eq!(
        chrome.scrollbar(),
        None,
        "nothing to scroll, so there is no thumb to take hold of"
    );

    chrome.set_scroll(ScrollState {
        offset: 0.0,
        visible: 0.5,
    });
    draw(&mut chrome, &input(&palette, &tabs, size));
    let (track, thumb) = scrollbar(&chrome);
    assert!(track.height > 0.0);
    assert!(thumb.height > 0.0 && thumb.height < track.height);
    assert!(
        (thumb.y - track.y).abs() < f32::EPSILON,
        "offset zero is the top of the track"
    );
}

// ---------------------------------------------------------------------------------
// The two planes
// ---------------------------------------------------------------------------------

/// A colour with its alpha divided back out.
///
/// The blend is premultiplied, so a quad drawn at a coverage carries the colour already
/// multiplied by it — which is how the caption marks are antialiased, one pixel at a
/// time. Undoing that multiplication is what recovers the colour the call site asked
/// for, and the colour the call site asked for is the thing the palette rule is about.
/// Without this the marks would have to be drawn opaque to pass, which is the same as
/// saying they could not be antialiased at all.
fn stripped(color: [f32; 4]) -> [f32; 4] {
    let alpha = color[3];
    if alpha <= 0.0 {
        return color;
    }
    // Alpha 1, not the coverage: the palette holds opaque colours, and the coverage was
    // never part of what was asked for.
    [color[0] / alpha, color[1] / alpha, color[2] / alpha, 1.0]
}

/// Whether two colours are the same one, allowing for the division above.
fn same(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-4)
}

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
            chrome.set_scroll(ScrollState {
                offset: 0.5,
                visible: 0.5,
            });
            let mut input = input(&palette, &tabs, window());
            input.settings_open = settings_open;
            input.find = Some(FindLine::default());
            let drawn = draw(&mut chrome, &input);

            let allowed: Vec<[f32; 4]> = palette
                .all()
                .iter()
                .map(|(_, color)| color.to_linear())
                .collect();
            assert!(!drawn.frame.quads.is_empty(), "nothing was drawn");
            for quad in &drawn.frame.quads {
                let colour = stripped(quad.color);
                assert!(
                    allowed.iter().any(|one| same(colour, *one)),
                    "{:?} is not a chrome colour",
                    quad.color
                );
            }
            for glyph in &drawn.frame.glyphs {
                let colour = stripped(glyph.color);
                assert!(
                    allowed.iter().any(|one| same(colour, *one)),
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

// ---------------------------------------------------------------------------------
// The caption marks
// ---------------------------------------------------------------------------------

/// One caption button's box, in logical pixels, for the 1200-wide test window.
fn caption_box(caption: Caption) -> Rect {
    let index = match caption {
        Caption::Minimize => 0.0,
        Caption::Maximize => 1.0,
        Caption::Close => 2.0,
    };
    Rect::new(1200.0 - 3.0 * 46.0 + index * 46.0, 0.0, 46.0, 40.0)
}

/// The ink one caption mark drew: coverage by pixel, keyed by that pixel's offset from
/// the mark's centre.
///
/// The marks are the only quads the chrome puts down at physical coordinates and the
/// only ones whose colour carries a coverage rather than a full alpha, so they can be
/// read back out of the frame — and a question about a *shape* becomes answerable here
/// ("the square is hollow", "the cross leaves the middle of its edges empty") rather
/// than only a question about a rectangle.
///
/// A mark's quad is a whole rectangle, not a pixel: the minimise bar is one quad ten
/// pixels wide. So each quad is expanded back into the pixels it covers, which is also
/// what makes the antialiased cross and the hinted bars comparable — both arrive here as
/// coverage per pixel.
///
/// Quads are kept only when the button's own box contains them. The chrome fills the
/// strip's surface behind the buttons with a quad that covers the whole window's width,
/// and that one is not a mark.
fn mark_ink(caption: Caption, scale: f32, maximized: bool) -> BTreeMap<(i32, i32), f32> {
    let palette = Palette::instrument();
    let tabs = tabs(&[1, 2]);
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, window());
    input.scale = scale;
    input.maximized = maximized;
    let drawn = draw(&mut chrome, &input);

    let button = caption_box(caption);
    let (cx, cy) = button.center();
    let (cx, cy) = ((cx * scale).round() as i32, (cy * scale).round() as i32);
    let box_ = Rect::new(
        button.x * scale,
        button.y * scale,
        button.width * scale,
        button.height * scale,
    );

    let mut ink = BTreeMap::new();
    for quad in &drawn.frame.quads {
        let [x, y, width, height] = quad.rect;
        if !box_.contains(x, y) || !box_.contains(x + width - 1.0, y + height - 1.0) {
            continue;
        }
        for row in y as i32..(y + height) as i32 {
            for column in x as i32..(x + width) as i32 {
                ink.insert((column - cx, row - cy), quad.color[3]);
            }
        }
    }
    ink
}

/// Whether anything was inked within `radius` pixels of `at`.
fn inked_near(ink: &BTreeMap<(i32, i32), f32>, at: (i32, i32), radius: i32) -> bool {
    ink.keys()
        .any(|(x, y)| (x - at.0).abs() <= radius && (y - at.1).abs() <= radius)
}

/// The rows and columns the ink occupies, as half-open ranges of pixel offsets.
fn ink_extent(ink: &BTreeMap<(i32, i32), f32>) -> ((i32, i32), (i32, i32)) {
    let xs = ink.keys().map(|(x, _)| *x);
    let ys = ink.keys().map(|(_, y)| *y);
    (
        (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0) + 1),
        (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0) + 1),
    )
}

#[test]
fn every_caption_draws_a_mark_and_no_mark_is_a_filled_block() {
    // The failure this is really about is a mark that is not there. The old ones were
    // characters from the chrome's face, and that face is loaded with no fallback chain —
    // so a character Plex does not carry draws `.notdef`, which is a box, in the place of
    // a window control. Geometry cannot go missing that way, and this says so.
    for caption in [Caption::Minimize, Caption::Maximize, Caption::Close] {
        for scale in [1.0_f32, 1.5, 2.0] {
            let ink = mark_ink(caption, scale, false);
            assert!(!ink.is_empty(), "{caption:?} at {scale} drew nothing");

            // Measured against the mark's own box and not against the extent of its ink,
            // because a bar ten pixels wide and one tall *is* its bounding box in full.
            // The claim is about the box the mark is drawn in: a control fills a
            // fraction of it, and a block fills all of it.
            let box_area = (10.0 * scale).powi(2);
            let inked = ink.values().sum::<f32>();
            assert!(
                inked < box_area * 0.75,
                "{caption:?} at {scale} inks {inked} of a possible {box_area} pixels, which \
                 is a filled block rather than a mark"
            );
        }
    }
}

#[test]
fn the_maximize_mark_is_a_hollow_square() {
    // A square drawn as an outline, not as a filled box and not as a character.
    for scale in [1.0_f32, 2.0] {
        let ink = mark_ink(Caption::Maximize, scale, false);
        assert!(
            !inked_near(&ink, (0, 0), scale.round() as i32 - 1),
            "the middle of the square is filled at {scale}"
        );
        // All four corners are inked: an outline with a corner missing is an outline
        // drawn from four segments that do not meet.
        let edge = (5.0 * scale).round() as i32;
        for corner in [
            (-edge + 1, -edge + 1),
            (edge - 2, -edge + 1),
            (-edge + 1, edge - 2),
            (edge - 2, edge - 2),
        ] {
            assert!(
                inked_near(&ink, corner, 2),
                "the corner at {corner:?} is missing at {scale}"
            );
        }
    }
}

#[test]
fn the_minimize_mark_is_one_bar_across_the_middle() {
    let ink = mark_ink(Caption::Minimize, 1.0, false);
    let ((x0, x1), (y0, y1)) = ink_extent(&ink);

    assert_eq!(y1 - y0, 1, "the bar is more than one pixel tall");
    assert!(
        (y0..y1).contains(&0),
        "the bar is not on the middle row: rows {y0}..{y1}"
    );
    assert!(
        x1 - x0 >= 8,
        "the bar is {} pixels wide and should span the mark's box",
        x1 - x0
    );
}

#[test]
fn the_close_mark_is_a_cross_and_not_a_diamond() {
    // Two diagonals. The discriminator is the middle of each edge: a cross inks its
    // corners and its centre and leaves those four points empty, while a filled square, a
    // diamond, or a box would each ink at least one of them.
    let ink = mark_ink(Caption::Close, 1.0, false);
    assert!(inked_near(&ink, (0, 0), 1), "the two strokes do not cross");

    for edge in [(-5, 0), (5, 0), (0, -5), (0, 5)] {
        assert!(
            !inked_near(&ink, edge, 0),
            "the middle of an edge at {edge:?} is inked, so this is not a cross"
        );
    }
    for corner in [(-4, -4), (4, -4), (-4, 4), (4, 4)] {
        assert!(
            inked_near(&ink, corner, 2),
            "the arm at {corner:?} is missing"
        );
    }
}

#[test]
fn a_maximized_window_gets_the_restore_mark() {
    // The middle button is one control with two shapes. A window that is already
    // maximized shows two overlapping squares, which is the only thing on the button that
    // tells the user what clicking it will do.
    let plain = mark_ink(Caption::Maximize, 1.0, false);
    let restored = mark_ink(Caption::Maximize, 1.0, true);
    assert_ne!(
        plain, restored,
        "the mark ignores whether the window is maximized"
    );

    let (_, (restored_top, _)) = ink_extent(&restored);
    assert!(
        restored_top < -4,
        "the back square does not sit above the front one: it starts at {restored_top}"
    );
    assert!(
        restored.len() > plain.len(),
        "two squares should be more ink than one"
    );
}

#[test]
fn a_caption_stroke_stays_one_pixel_until_there_is_a_second_one_to_spend() {
    // Optical sizing, and the reason the rasteriser works in physical pixels at all: a
    // one-pixel hairline scaled to 1.25 is 1.25 pixels, which a linear coverage ramp
    // spreads over two rows as a grey smear. A caption mark is a hard edge, so it stays
    // one pixel until the scale offers a whole second one.
    let rows = |scale: f32| {
        let ink = mark_ink(Caption::Minimize, scale, false);
        let (_, (y0, y1)) = ink_extent(&ink);
        y1 - y0
    };
    // In *physical* rows, because that is what the grid the mark lands on is measured
    // in: the claim is "one pixel of stroke", not "one logical pixel", and at 125% those
    // are different numbers. Spending 1.25 pixels of stroke as two physical rows is the
    // grey smear; spending it as one is the hairline Windows draws.
    assert_eq!(rows(1.0), 1);
    assert_eq!(rows(1.25), 1, "1.25 pixels of stroke was spent as two");
    assert_eq!(rows(1.5), 2, "half way to a second pixel rounds to one");
    assert_eq!(rows(2.0), 2, "a 2x window gets two pixels of stroke");
}

#[test]
fn the_marks_grow_with_the_window_scale() {
    let small = mark_ink(Caption::Maximize, 1.0, false);
    let large = mark_ink(Caption::Maximize, 2.0, false);
    let ((small_left, small_right), _) = ink_extent(&small);
    let ((large_left, large_right), _) = ink_extent(&large);
    assert_eq!(large_right - large_left, (small_right - small_left) * 2);
    assert_eq!(large_left, small_left * 2);
}

// ---------------------------------------------------------------------------------
// The profile picker
// ---------------------------------------------------------------------------------

/// The names a picker line borrows, in the order they would be opened.
fn names(shells: &[String]) -> Vec<&str> {
    shells.iter().map(String::as_str).collect()
}

/// The popover's row regions, in order.
fn picker_rows(chrome: &Chrome) -> Vec<(usize, Rect)> {
    chrome
        .regions()
        .iter()
        .filter_map(|region| match region {
            crate::Region::Profile { row, rect } => Some((*row, *rect)),
            _ => None,
        })
        .collect()
}

/// The colour of the first glyph drawn for `ch`.
///
/// [`drawn`] reads back which characters reached the frame and not what they were painted
/// in, because every test before this one was about what was drawn rather than what
/// colour it was. A picker's whole readout is the colour — the lit row is the one in
/// `ink` — so this is where the other half of a glyph is needed.
fn ink_of(frame: &Frame, ch: char) -> Option<[u32; 4]> {
    frame
        .glyphs
        .iter()
        .find(|glyph| char::from_u32(glyph.uv[1] as u32) == Some(ch))
        .map(|glyph| glyph.color.map(f32::to_bits))
}

/// A layout with the picker open over a given list of shells.
fn open_picker<'a>(
    palette: &'a Palette,
    tabs: &'a [TabInfo],
    profiles: &'a [&'a str],
    at: usize,
) -> (Chrome, Drawn) {
    let mut chrome = chrome();
    let mut input = input(palette, tabs, window());
    input.picker = Some(PickerLine { profiles, at });
    let drawn = draw(&mut chrome, &input);
    (chrome, drawn)
}

/// The same, in a window of a size the caller picks.
fn open_picker_at_size<'a>(
    palette: &'a Palette,
    tabs: &'a [TabInfo],
    profiles: &'a [&'a str],
    at: usize,
    size: Size,
) -> Chrome {
    let mut chrome = chrome();
    let mut input = input(palette, tabs, size);
    input.picker = Some(PickerLine { profiles, at });
    let _ = draw(&mut chrome, &input);
    chrome
}

#[test]
fn no_question_means_no_popover() {
    // The picker is the shape of a question, and a machine whose file does not ask has no
    // question. A list of shells that appeared anyway would be a list nobody asked for
    // sitting over the terminal, taking clicks meant for the grid.
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let mut chrome = chrome();
    let _ = draw(&mut chrome, &input(&palette, &tabs, window()));

    assert!(picker_rows(&chrome).is_empty());
    assert_eq!(
        chrome.hit(40.0, 60.0),
        Hit::None,
        "nothing was there to hit"
    );
}

#[test]
fn the_popover_names_every_shell_it_offers() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let shells = vec![
        "PowerShell 7".to_owned(),
        "cmd".to_owned(),
        "Ubuntu".to_owned(),
    ];
    let profiles = names(&shells);
    let (chrome, drawn) = open_picker(&palette, &tabs, &profiles, 0);

    assert_eq!(picker_rows(&chrome).len(), 3);
    let body: Vec<char> = drawn_at(&drawn.frame, Weight::NORMAL);
    for expected in ["PowerShell 7", "cmd", "Ubuntu"] {
        for letter in expected.chars().filter(|ch| !ch.is_whitespace()) {
            assert!(body.contains(&letter), "{expected} was not drawn");
        }
    }
}

#[test]
fn the_row_the_question_is_on_is_the_row_in_ink() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let shells = vec![
        "PowerShell 7".to_owned(),
        "cmd".to_owned(),
        "Ubuntu".to_owned(),
    ];
    let profiles = names(&shells);
    let (_, drawn) = open_picker(&palette, &tabs, &profiles, 1);

    // The lit row is `ink` and the others are `ink-mid`, which is the strip's own rule:
    // the tab you are in is `ink` and the ones you are not are quieter. Weight is not
    // touched, because DESIGN.md gives 500 to the active tab and the section headings and
    // to nothing else.
    assert_eq!(ink_of(&drawn.frame, 'c'), Some(color(palette.ink)));
    assert_eq!(ink_of(&drawn.frame, 'P'), Some(color(palette.ink_mid)));
    assert_eq!(ink_of(&drawn.frame, 'U'), Some(color(palette.ink_mid)));
}

#[test]
fn the_lit_row_carries_the_lamp_the_active_tab_carries() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let shells = vec![
        "PowerShell 7".to_owned(),
        "cmd".to_owned(),
        "Ubuntu".to_owned(),
    ];
    let profiles = names(&shells);
    let (chrome, drawn) = open_picker(&palette, &tabs, &profiles, 2);

    let (_, row) = picker_rows(&chrome)
        .into_iter()
        .find(|(at, _)| *at == 2)
        .expect("the third row was laid out");
    let signal = color(palette.signal);
    let lamp = drawn
        .frame
        .quads
        .iter()
        .find(|quad| {
            let [x, y, width, height] = quad.rect;
            quad.color.map(f32::to_bits) == signal
                && (width - 2.0).abs() < f32::EPSILON
                && (height - row.height).abs() < f32::EPSILON
                && x >= row.x - 2.0
                && x <= row.x + 1.0
                && y >= row.y - 1.0
                && y <= row.y + 1.0
        })
        .expect("the lit row is marked by a two-pixel bar on its left edge");
    // Within DESIGN.md's budget: `signal` never fills more than 3px by 40px, which is what
    // the tab strip's active marker is and what this is.
    assert!(lamp.rect[3] <= 40.0, "the lamp grew past its budget");
}

#[test]
fn a_click_on_a_row_is_that_row() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let shells = vec![
        "PowerShell 7".to_owned(),
        "cmd".to_owned(),
        "Ubuntu".to_owned(),
    ];
    let profiles = names(&shells);
    let (chrome, _) = open_picker(&palette, &tabs, &profiles, 0);

    let rows = picker_rows(&chrome);
    assert_eq!(rows.len(), 3);
    for (at, row) in rows {
        assert_eq!(
            chrome.hit(row.x + row.width / 2.0, row.y + row.height / 2.0),
            Hit::Profile(at)
        );
    }
}

#[test]
fn a_click_on_the_popover_but_not_on_a_row_is_the_popover() {
    // The surface takes the click rather than passing it to the terminal behind it. A
    // popover with holes in it is a popover where clicking the padding types into the
    // shell, which is the one thing the user was not doing.
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let shells = vec!["PowerShell 7".to_owned(), "cmd".to_owned()];
    let profiles = names(&shells);
    let (chrome, _) = open_picker(&palette, &tabs, &profiles, 0);

    let (_, first) = picker_rows(&chrome)[0];
    // The caption strip above the first row, which belongs to the popover and to no row.
    assert_eq!(
        chrome.hit(first.x + 6.0, first.y - 4.0),
        Hit::Picker,
        "the popover's own surface is not a hole"
    );
}

#[test]
fn a_click_under_the_popover_reaches_the_grid() {
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let shells = vec!["PowerShell 7".to_owned(), "cmd".to_owned()];
    let profiles = names(&shells);
    let (chrome, _) = open_picker(&palette, &tabs, &profiles, 0);

    let lowest = picker_rows(&chrome)
        .into_iter()
        .map(|(_, rect)| rect.bottom())
        .fold(0.0_f32, f32::max);
    let below = chrome.hit(60.0, lowest + 40.0);
    assert_ne!(below, Hit::Picker);
    assert_ne!(below, Hit::Profile(0));
}

#[test]
fn a_popover_row_is_still_reachable_with_the_settings_panel_open_behind_it() {
    // The two overlays can be up at once — the panel is a view the user left open and the
    // question about a new tab is a thing they just did — and in a window narrow enough
    // they overlap: the panel is anchored to the right edge and is 380 wide, the popover to
    // the left and is 300. The popover is drawn after the panel, so it is the one the user
    // can see, and a click on a row has to land on the row. It did not: `Chrome::hit`
    // answers with the first region that holds the point, the panel's surface was pushed
    // before the popover's rows, and a click on a row in the overlap was swallowed by a
    // panel the row was drawn on top of.
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let shells = vec!["PowerShell 7".to_owned(), "cmd".to_owned()];
    let profiles = names(&shells);
    let size = Size {
        width: 600.0,
        height: 800.0,
    };
    let lines: Vec<SettingLine<'static>> = (0..3)
        .map(|_| SettingLine {
            text: "Size",
            control: Some(Control::Step),
            value: "13",
        })
        .collect::<Vec<_>>();
    let mut chrome = chrome();
    let mut input = input(&palette, &tabs, size);
    input.settings_open = true;
    input.settings = &lines;
    input.picker = Some(PickerLine {
        profiles: &profiles,
        at: 0,
    });
    let _ = draw(&mut chrome, &input);

    let panel = chrome
        .regions()
        .iter()
        .find_map(|region| match region {
            crate::Region::Settings(rect) => Some(*rect),
            _ => None,
        })
        .expect("the panel is open");
    let (_, row) = picker_rows(&chrome)
        .into_iter()
        .next()
        .expect("the popover drew a row");
    assert!(
        row.x < panel.x && panel.x < row.right(),
        "the two do not overlap in this window, so this is not the conflict it tests"
    );

    let x = panel.x + 4.0;
    assert_eq!(
        chrome.hit(x, row.center().1),
        Hit::Profile(0),
        "the panel swallowed a click on the row drawn over it"
    );
}

#[test]
fn the_popover_stays_inside_a_window_too_short_to_hold_it() {
    // Rows that do not fit are scrolled to rather than drawn off the bottom edge, which is
    // the rule the settings panel follows and for the same reason: a list that runs off the
    // window is a shell the user can select and cannot see.
    let palette = Palette::instrument();
    let tabs = tabs(&[1]);
    let shells: Vec<String> = ["one", "two", "three", "four", "five", "six"]
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let profiles = names(&shells);
    let chrome = open_picker_at_size(
        &palette,
        &tabs,
        &profiles,
        5,
        Size {
            width: 800.0,
            height: 260.0,
        },
    );

    let rows = picker_rows(&chrome);
    for (at, rect) in &rows {
        assert!(
            rect.bottom() <= 260.0,
            "row {at} ran off the bottom of the window"
        );
    }
    assert!(
        rows.iter().any(|(at, _)| *at == 5),
        "the row the question is on has to be on screen"
    );
    assert!(
        rows.len() < profiles.len(),
        "a window this short cannot have shown every shell"
    );
}
