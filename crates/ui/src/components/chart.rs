use gpui::{
    App, Bounds, Hsla, IntoElement, PathBuilder, Pixels, RenderOnce, Window, canvas, div,
    linear_color_stop, linear_gradient, point, prelude::*, px,
};

use crate::Theme;

/// One line on a [`Sparkline`].
#[derive(Debug, Clone)]
pub struct Series {
    values: Vec<f32>,
    color: Hsla,
}

impl Series {
    #[must_use]
    pub const fn new(values: Vec<f32>, color: Hsla) -> Self {
        Self { values, color }
    }
}

/// A small live chart of one or more series.
///
/// Drawn right-aligned, so new samples enter at the right edge and scroll
/// left. Each series is a 1.5 px stroke over a gradient wash that fades to
/// nothing at the baseline.
#[derive(IntoElement)]
#[must_use]
pub struct Sparkline {
    series: Vec<Series>,
    /// Sample slots across the full width.
    capacity: usize,
    /// Fixed top of the scale; `None` scales to the largest value shown.
    max: Option<f32>,
    height: f32,
}

impl Sparkline {
    pub const fn new(capacity: usize) -> Self {
        Self {
            series: Vec::new(),
            capacity,
            max: None,
            height: 56.0,
        }
    }

    pub fn series(mut self, series: Series) -> Self {
        self.series.push(series);
        self
    }

    pub const fn max(mut self, max: f32) -> Self {
        self.max = Some(max);
        self
    }

    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
}

impl RenderOnce for Sparkline {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let grid = Theme::of(cx).border;
        let Self {
            series,
            capacity,
            max,
            height,
        } = self;
        let top = max.unwrap_or_else(|| {
            series
                .iter()
                .flat_map(|s| s.values.iter().copied())
                .fold(0.0, f32::max)
        });
        // Headroom so a flat-out series doesn't ride the top edge, and a
        // floor so an idle one stays a flat line at the bottom.
        let scale = (top * 1.15).max(f32::EPSILON);
        div().w_full().h(px(height)).child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    paint_grid(bounds, grid, window);
                    for s in &series {
                        paint_series(bounds, s, capacity, scale, window);
                    }
                },
            )
            .size_full(),
        )
    }
}

fn paint_grid(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let (left, top, width, height) = rect(bounds);
    for fraction in [0.0, 0.5] {
        let y = height.mul_add(fraction, top) + 0.5;
        let mut path = PathBuilder::stroke(px(1.0)).dash_array(&[px(2.0), px(3.0)]);
        path.move_to(point(px(left), px(y)));
        path.line_to(point(px(left + width), px(y)));
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
    let mut base = PathBuilder::stroke(px(1.0));
    let y = top + height - 0.5;
    base.move_to(point(px(left), px(y)));
    base.line_to(point(px(left + width), px(y)));
    if let Ok(path) = base.build() {
        window.paint_path(path, color);
    }
}

fn paint_series(
    bounds: Bounds<Pixels>,
    series: &Series,
    capacity: usize,
    scale: f32,
    window: &mut Window,
) {
    let points = layout(rect(bounds), &series.values, capacity, scale);
    let (Some(&(first_x, _)), Some(&(last_x, _))) = (points.first(), points.last()) else {
        return;
    };
    let (_, top, _, height) = rect(bounds);
    let bottom = top + height;

    let mut area = PathBuilder::fill();
    area.move_to(point(px(first_x), px(bottom)));
    for &(x, y) in &points {
        area.line_to(point(px(x), px(y)));
    }
    area.line_to(point(px(last_x), px(bottom)));
    area.close();
    if let Ok(path) = area.build() {
        window.paint_path(
            path,
            linear_gradient(
                180.0,
                linear_color_stop(series.color.opacity(0.24), 0.0),
                linear_color_stop(series.color.opacity(0.0), 1.0),
            ),
        );
    }

    let mut line = PathBuilder::stroke(px(1.5));
    for (ix, &(x, y)) in points.iter().enumerate() {
        if ix == 0 {
            line.move_to(point(px(x), px(y)));
        } else {
            line.line_to(point(px(x), px(y)));
        }
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, series.color);
    }
}

fn rect(bounds: Bounds<Pixels>) -> (f32, f32, f32, f32) {
    (
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}

/// Right-aligned sample positions. The newest value sits on the right edge;
/// older ones step left by `width / (capacity − 1)`. Values beyond capacity
/// are dropped from the left; a 1.5 px inset keeps the stroke inside.
fn layout(
    (left, top, width, height): (f32, f32, f32, f32),
    values: &[f32],
    capacity: usize,
    scale: f32,
) -> Vec<(f32, f32)> {
    let capacity = capacity.max(2);
    let shown = values
        .get(values.len().saturating_sub(capacity)..)
        .unwrap_or_default();
    let step = width / slot(capacity.saturating_sub(1));
    let inset: f32 = 1.5;
    let usable = inset.mul_add(-2.0, height).max(0.0);
    let right = left + width;
    let newest = shown.len().saturating_sub(1);
    shown
        .iter()
        .enumerate()
        .map(|(ix, &v)| {
            let age = slot(newest.saturating_sub(ix));
            let x = step.mul_add(-age, right).max(left);
            let fraction = (v / scale).clamp(0.0, 1.0);
            (x, usable.mul_add(-fraction, top + height - inset))
        })
        .collect()
}

/// A small count as `f32` (exact up to 2^16, which bounds every chart here).
fn slot(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use super::layout;

    #[test]
    fn newest_sample_is_on_the_right_edge() {
        let pts = layout((0.0, 0.0, 100.0, 50.0), &[0.0, 5.0, 10.0], 11, 10.0);
        assert_eq!(pts.len(), 3);
        let (x, y) = pts[2];
        assert!((x - 100.0).abs() < 1e-4);
        assert!((y - 1.5).abs() < 1e-4, "full scale touches the top inset");
        assert!((pts[1].0 - 90.0).abs() < 1e-4);
        assert!((pts[0].1 - 48.5).abs() < 1e-4, "zero sits on the baseline");
    }

    #[test]
    fn overflowing_history_keeps_the_newest() {
        let values: Vec<f32> = (0..20u8).map(f32::from).collect();
        let pts = layout((0.0, 0.0, 100.0, 50.0), &values, 5, 19.0);
        assert_eq!(pts.len(), 5);
        assert!((pts[0].0).abs() < 1e-4);
    }
}
