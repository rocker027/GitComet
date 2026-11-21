use crate::theme::{AppTheme, hsla_from_hue_fraction, with_alpha};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Bounds, Div, PaintQuad, Pixels, Rgba, Window, div, point, px, size};
use palette::IntoColor;
use rustc_hash::FxHasher;

/// Deterministic identity color for an author name. Uses the same hue recipe
/// as the history graph lanes (hash-driven hue, fixed saturation,
/// theme-dependent lightness) so avatars read as part of the same palette.
pub fn author_color(theme: AppTheme, name: &str) -> Rgba {
    let hue = (author_name_hash(name) % 360) as f32 / 360.0;
    let light = if theme.is_dark { 0.62 } else { 0.34 };
    hsla_from_hue_fraction(hue, 0.60, light, 1.0).into_color()
}

fn author_name_hash(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    name.hash(&mut hasher);
    hasher.finish()
}

/// Rows in an author mark, top to bottom.
pub const AUTHOR_MARK_ROWS: usize = 4;
/// Columns in an author mark. Bit `x` of a row inks column `x`.
const AUTHOR_MARK_COLS: u32 = 5;
/// Width of one mark cell as a fraction of the circle it sits in.
const AUTHOR_MARK_CELL_RATIO: f32 = 0.105;

/// Deterministic identity mark for an author name: four rows of five cells,
/// each row mirrored about its centre. Authors who share initials still get
/// different marks, and painting one needs no text shaping.
pub fn author_mark(name: &str) -> [u8; AUTHOR_MARK_ROWS] {
    // Each row has ink and a gap. Mirror symmetry keeps the mark legible
    // even when a compact history row has only one device pixel per cell.
    const ROWS: [u8; 6] = [0b00100, 0b01010, 0b10001, 0b01110, 0b10101, 0b11011];
    let mut seed = author_name_hash(name);
    std::array::from_fn(|_| {
        let row = ROWS[(seed % ROWS.len() as u64) as usize];
        seed /= ROWS.len() as u64;
        row
    })
}

/// Paints `mark` in `ink`, centred in the circle `bounds` and shifted up and
/// left by `inset` (the history graph clears its merge badge that way). Cells
/// are whole device pixels sized from the circle, so every surface that shows
/// an author draws the same proportions.
pub fn paint_author_mark(
    bounds: Bounds<Pixels>,
    inset: Pixels,
    mark: [u8; AUTHOR_MARK_ROWS],
    ink: Rgba,
    window: &mut Window,
) {
    let scale_factor = window.scale_factor();
    // Halves round up and a cell never drops below one device pixel.
    let cell_device_px = (f32::from(bounds.size.width * AUTHOR_MARK_CELL_RATIO) * scale_factor)
        .round()
        .max(1.0);
    let cell = px(cell_device_px / scale_factor);
    let origin = window.pixel_snap_point(point(
        bounds.center().x - cell * (AUTHOR_MARK_COLS as f32 * 0.5) - inset,
        bounds.center().y - cell * (AUTHOR_MARK_ROWS as f32 * 0.5) - inset,
    ));
    // Adjacent cells in a row share a quad. Device-pixel cells keep the mark
    // crisp without shaping text or allocating on each animation frame.
    for (y, bits) in mark.into_iter().enumerate() {
        let mut x = 0;
        while x < AUTHOR_MARK_COLS {
            if bits & (1 << x) == 0 {
                x += 1;
                continue;
            }
            let start = x;
            while x < AUTHOR_MARK_COLS && bits & (1 << x) != 0 {
                x += 1;
            }
            let run = Bounds::new(
                point(origin.x + cell * start as f32, origin.y + cell * y as f32),
                size(cell * (x - start) as f32, cell),
            );
            window.paint_quad(PaintQuad::new(run).background(ink));
        }
    }
}

pub const AVATAR_DIAMETER_PX: f32 = 16.0;

/// Y origin to hand `ShapedLine::paint` so capital-letter initials sit
/// visually centered in a circle. gpui places the baseline at
/// `origin_y + (line_height - ascent - descent) / 2 + ascent`, while the ink
/// of capitals spans `[baseline - cap_height, baseline]` — so the baseline
/// must land at `circle_center + cap_height / 2`, not at the line-box center.
pub fn initials_paint_origin_y(
    circle_top: Pixels,
    diameter: Pixels,
    line_height: Pixels,
    ascent: Pixels,
    descent: Pixels,
    cap_height: Pixels,
) -> Pixels {
    let baseline_in_line = (line_height - ascent - descent) * 0.5 + ascent;
    circle_top + diameter * 0.5 + cap_height * 0.5 - baseline_in_line
}

/// Tinted circle + author mark, for retained-mode UIs (commit details,
/// popovers). The history table paints the same mark directly on its row
/// canvas, so an author looks the same in the graph and beside their name.
pub fn author_avatar(theme: AppTheme, scale: impl Into<UiScale>, name: &str) -> Div {
    author_avatar_sized(theme, scale.into().px(AVATAR_DIAMETER_PX), name)
}

/// [`author_avatar`] at `diameter`, its mark scaled with it.
pub fn author_avatar_sized(theme: AppTheme, diameter: Pixels, name: &str) -> Div {
    let color = author_color(theme, name);
    let mark = author_mark(name);
    div()
        .flex_none()
        .w(diameter)
        .h(diameter)
        .rounded(diameter * 0.5)
        .bg(with_alpha(color, 0.22))
        .child(
            gpui::canvas(
                |_bounds, _window, _cx| {},
                move |bounds, (), window, _cx| {
                    paint_author_mark(bounds, px(0.0), mark, color, window);
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::AppTheme;
    use palette::{Hsla, IntoColor};

    /// Distinct authors must land on distinct hues, not four shades of one.
    #[test]
    fn author_colors_spread_across_the_hue_wheel() {
        let theme = AppTheme::gitcomet_dark();
        let names = [
            "Ada Lovelace",
            "Grace Hopper",
            "Alan Turing",
            "Barbara Liskov",
        ];
        let mut hues = names.map(|name| {
            let color: Hsla = author_color(theme, name).into_color();
            color.hue.into_positive_degrees()
        });
        hues.sort_by(f32::total_cmp);

        // Under the bug this guards, every hue lands under one degree, so the
        // whole spread collapses and adjacent authors become indistinguishable.
        let spread = hues[hues.len() - 1] - hues[0];
        assert!(
            spread > 90.0,
            "author hues span only {spread}°, so they have collapsed onto one colour: {hues:?}"
        );
        for pair in hues.windows(2) {
            assert!(
                pair[1] - pair[0] > 1.0,
                "author hues {:?} and {:?} are within a degree of each other: {hues:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn author_marks_are_deterministic_and_tell_shared_initials_apart() {
        // Both names abbreviate to "AL".
        assert_eq!(author_mark("Ada Lovelace"), author_mark("Ada Lovelace"));
        assert_ne!(author_mark("Ada Lovelace"), author_mark("Alan Lane"));
    }

    #[test]
    fn author_mark_rows_have_ink_fit_five_columns_and_mirror() {
        for name in ["", "符文", "rune", "Ada Lovelace", "dependabot[bot]"] {
            for row in author_mark(name) {
                assert!(
                    (1..1 << AUTHOR_MARK_COLS).contains(&row),
                    "{name}: {row:#07b}"
                );
                let mirrored = row.reverse_bits() >> (u8::BITS - AUTHOR_MARK_COLS);
                assert_eq!(row, mirrored, "{name}: {row:#07b}");
            }
        }
    }

    const AVATAR_NAME: &str = "Ada Lovelace";

    struct AvatarView {
        theme: AppTheme,
    }

    impl gpui::Render for AvatarView {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            // An odd offset keeps the circle off the device-pixel grid.
            div().flex().p(px(7.25)).child(
                author_avatar(self.theme, UiScale::from_percent(100), AVATAR_NAME)
                    .debug_selector(|| "avatar".to_string()),
            )
        }
    }

    /// The avatar beside a name paints the same mark as the history graph
    /// node, in the author colour, centred in its circle.
    #[gpui::test]
    fn avatar_paints_the_author_mark_centred_in_its_circle(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::gitcomet_dark();
        let (_view, cx) = cx.add_window_view(|_, _| AvatarView { theme });
        let ink: gpui::Background = author_color(theme, AVATAR_NAME).into();
        let mark = author_mark(AVATAR_NAME);
        let first_ink = mark.iter().map(|row| row.trailing_zeros()).min().unwrap();
        for device_scale in [1.0, 1.5, 2.0] {
            cx.update(|window, _| window.set_scale_factor(device_scale));
            crate::test_support::refresh_and_draw(cx);
            let circle = cx.debug_bounds("avatar").expect("avatar must be drawn");
            // Painted quads are in device pixels; layout bounds are logical.
            let runs: Vec<_> = cx.update(|window, _| {
                window
                    .painted_quads()
                    .into_iter()
                    .filter(|quad| quad.background == ink)
                    .map(|quad| quad.bounds)
                    .collect()
            });
            assert!(!runs.is_empty(), "{device_scale}x: no mark painted");
            let cell = runs[0].size.height.0;
            assert!(
                cell >= 1.0 && cell.fract() == 0.0,
                "{device_scale}x: {cell}"
            );
            let left = runs
                .iter()
                .map(|run| run.left().0)
                .reduce(f32::min)
                .unwrap();
            let top = runs.iter().map(|run| run.top().0).reduce(f32::min).unwrap();
            let mut painted = [0u8; AUTHOR_MARK_ROWS];
            for run in &runs {
                let x = ((run.left().0 - left) / cell).round() as u32;
                let y = ((run.top().0 - top) / cell).round() as usize;
                let width = (run.size.width.0 / cell).round() as u32;
                painted[y] |= ((1 << width) - 1) << x;
            }
            // A mark without ink in its outer columns paints a narrower extent.
            assert_eq!(painted, mark.map(|row| row >> first_ink), "{device_scale}x");
            let mark_center_x = left + cell * (AUTHOR_MARK_COLS as f32 * 0.5 - first_ink as f32);
            let mark_center_y = top + cell * (AUTHOR_MARK_ROWS as f32 * 0.5);
            let circle_center = circle.center();
            for (mark_center, circle_center) in [
                (mark_center_x, f32::from(circle_center.x) * device_scale),
                (mark_center_y, f32::from(circle_center.y) * device_scale),
            ] {
                assert!(
                    (mark_center - circle_center).abs() <= 0.5,
                    "{device_scale}x: mark centre {mark_center} vs circle {circle_center}"
                );
            }
        }
    }

    #[test]
    fn author_color_is_deterministic_and_name_sensitive() {
        let theme = AppTheme::gitcomet_dark();
        assert_eq!(
            author_color(theme, "Sampo Kivistö"),
            author_color(theme, "Sampo Kivistö")
        );
        assert_ne!(
            author_color(theme, "Sampo Kivistö"),
            author_color(theme, "Roope Airinen")
        );
    }

    #[test]
    fn initials_origin_places_baseline_at_center_plus_half_cap_height() {
        // line box 16px, ascent 8, descent 2, cap height 6, circle top 0 / d 16.
        let origin_y =
            initials_paint_origin_y(px(0.0), px(16.0), px(16.0), px(8.0), px(2.0), px(6.0));
        // gpui paints the baseline at origin + (lh - ascent - descent)/2 + ascent.
        let baseline = origin_y + (px(16.0) - px(8.0) - px(2.0)) * 0.5 + px(8.0);
        // Circle center (8) + cap/2 (3): capital ink [5, 11] centers on 8.
        assert_eq!(baseline, px(11.0));
    }

    #[test]
    fn initials_origin_is_line_height_invariant() {
        // The same visual placement must come out regardless of the line box
        // the style happens to resolve (the term cancels out of the math).
        let a = initials_paint_origin_y(px(0.0), px(16.0), px(12.0), px(8.0), px(2.0), px(6.0))
            + (px(12.0) - px(8.0) - px(2.0)) * 0.5
            + px(8.0);
        let b = initials_paint_origin_y(px(0.0), px(16.0), px(20.0), px(8.0), px(2.0), px(6.0))
            + (px(20.0) - px(8.0) - px(2.0)) * 0.5
            + px(8.0);
        assert_eq!(a, b);
    }
}
