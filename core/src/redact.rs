//! Irreversible "frosted glass" over secret regions.
//!
//! AIDEV-NOTE: privacy-critical. A real Gaussian blur of text can be reversed, so the frost is random noise
//! tinted with two averaged colours. No pixel of the result depends on the shape of the text beneath it.

use crate::detect::Rect;

pub fn frost(rgba: &mut [u8], width: usize, height: usize, stride: usize, rects: &[Rect]) {
    for r in rects {
        // Pad: OCR boxes clip ascenders/descenders and are a little loose horizontally.
        let pad = (r.h * height as f64 * 0.22).max(4.0);
        let x0 = ((r.x * width as f64 - pad).floor().max(0.0) as usize).min(width);
        let y0 = ((r.y * height as f64 - pad).floor().max(0.0) as usize).min(height);
        let x1 = (((r.x + r.w) * width as f64 + pad).ceil().max(0.0) as usize).min(width);
        let y1 = (((r.y + r.h) * height as f64 + pad).ceil().max(0.0) as usize).min(height);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let px = |x: usize, y: usize| y * stride + x * 4;
        let mut ink = [0u64; 3];
        let mut n = 0u64;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = px(x, y);
                for c in 0..3 {
                    ink[c] += rgba[p + c] as u64;
                }
                n += 1;
            }
        }
        let mean: [f64; 3] = std::array::from_fn(|c| ink[c] as f64 / n as f64);
        let luma = 0.299 * mean[0] + 0.587 * mean[1] + 0.114 * mean[2];
        let shade = if luma > 128.0 { 0.0 } else { 255.0 };

        let cell = (((y1 - y0) / 2).max(6)) as f64;
        let (gw, gh) = (((x1 - x0) as f64 / cell).ceil() as usize + 2, ((y1 - y0) as f64 / cell).ceil() as usize + 2);
        let grid: Vec<f64> = (0..gw * gh).map(|_| fastrand::f64()).collect();
        for y in y0..y1 {
            let fy = (y - y0) as f64 / cell;
            let (iy, ty) = (fy as usize, fy.fract());
            for x in x0..x1 {
                let fx = (x - x0) as f64 / cell;
                let (ix, tx) = (fx as usize, fx.fract());
                let g = |a: usize, b: usize| grid[b * gw + a];
                let top = g(ix, iy) * (1.0 - tx) + g(ix + 1, iy) * tx;
                let bot = g(ix, iy + 1) * (1.0 - tx) + g(ix + 1, iy + 1) * tx;
                let k = 0.06 + 0.26 * (top * (1.0 - ty) + bot * ty);
                let p = px(x, y);
                for c in 0..3 {
                    rgba[p + c] = (mean[c] * (1.0 - k) + shade * k).round() as u8;
                }
                rgba[p + 3] = 255;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two different texts with equal averages must be indistinguishable after frosting with the same noise.
    #[test]
    fn output_is_independent_of_text_shape() {
        let (w, h) = (120, 40);
        let paint = |phase: usize| {
            let mut v = vec![255u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    if (x + phase) % 2 == 0 {
                        v[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(0);
                    }
                }
            }
            v
        };
        let r = [Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }];
        let (mut a, mut b) = (paint(0), paint(1));
        fastrand::seed(7);
        frost(&mut a, w, h, w * 4, &r);
        fastrand::seed(7);
        frost(&mut b, w, h, w * 4, &r);
        assert_eq!(a, b);
    }

    #[test]
    fn respects_bounds_and_stride() {
        let (w, h, stride) = (10, 10, 48);
        let mut v = vec![9u8; stride * h];
        frost(&mut v, w, h, stride, &[Rect { x: -0.5, y: 0.9, w: 3.0, h: 3.0 }]);
        assert!((0..h).all(|y| v[y * stride + w * 4..(y + 1) * stride].iter().all(|&b| b == 9)));
    }
}
