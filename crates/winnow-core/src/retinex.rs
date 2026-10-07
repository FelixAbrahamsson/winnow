//! Multi-scale Retinex: evens out uneven illumination by dividing each pixel's
//! luminance by blurred versions of itself (in log space), then stretching the
//! result to the full 0..255 range. Applied to luminance only; colour pixels
//! are scaled by the same ratio so hues are kept.
//!
//! The blurred illumination is smooth, so it's estimated on a downscaled copy
//! (short side <= `WORK_SIZE`) and upsampled bilinearly — big images stay fast.

/// Blur scales, as fractions of the image's shorter side.
const SCALES: [f32; 3] = [0.04, 0.2, 0.7];
/// Percentiles the result is stretched between.
const LOW_PCT: f32 = 0.01;
const HIGH_PCT: f32 = 0.99;
/// Max short side of the image the illumination is estimated on.
const WORK_SIZE: usize = 512;

/// Apply multi-scale Retinex in place to packed 8-bit RGB(A) pixels.
pub fn apply(data: &mut [u8], width: usize, height: usize, rowstride: usize, n_channels: usize) {
    if width == 0 || height == 0 {
        return;
    }
    let luma: Vec<f32> = (0..height)
        .flat_map(|y| {
            let row = &data[y * rowstride..y * rowstride + width * n_channels];
            row.chunks_exact(n_channels)
                .map(|px| 0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32)
                .collect::<Vec<_>>()
        })
        .collect();

    // Mean over scales of log(blurred luminance), at low resolution.
    let f = width.min(height).div_ceil(WORK_SIZE);
    let (small, sw, sh) = downscale(&luma, width, height, f);
    let mut illum = vec![0f32; small.len()];
    for frac in SCALES {
        let mut blurred = small.clone();
        gaussian_blur(&mut blurred, sw, sh, (frac * sw.min(sh) as f32).max(1.0));
        for (i, &b) in illum.iter_mut().zip(&blurred) {
            *i += (b + 1.0).ln() / SCALES.len() as f32;
        }
    }
    let r: Vec<f32> = (0..width * height)
        .map(|i| (luma[i] + 1.0).ln() - bilinear(&illum, sw, sh, f, i % width, i / width))
        .collect();

    let (lo, hi) = percentiles(&r);
    let span = (hi - lo).max(1e-6);
    for y in 0..height {
        let row = &mut data[y * rowstride..y * rowstride + width * n_channels];
        for (x, px) in row.chunks_exact_mut(n_channels).enumerate() {
            let i = y * width + x;
            let out = ((r[i] - lo) / span).clamp(0.0, 1.0) * 255.0;
            if luma[i] < 1.0 {
                px[..3].fill(out.round() as u8);
            } else {
                let k = out / luma[i];
                for c in &mut px[..3] {
                    *c = (*c as f32 * k).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// Box-average `img` down by an integer factor `f` (partial edge blocks too).
fn downscale(img: &[f32], width: usize, height: usize, f: usize) -> (Vec<f32>, usize, usize) {
    if f <= 1 {
        return (img.to_vec(), width, height);
    }
    let (sw, sh) = (width.div_ceil(f), height.div_ceil(f));
    let mut sum = vec![0f32; sw * sh];
    let mut count = vec![0u32; sw * sh];
    for (i, &v) in img.iter().enumerate() {
        let j = (i / width / f) * sw + (i % width) / f;
        sum[j] += v;
        count[j] += 1;
    }
    for (s, &c) in sum.iter_mut().zip(&count) {
        *s /= c as f32;
    }
    (sum, sw, sh)
}

/// Sample a map downscaled by `f` at full-resolution pixel (x, y).
fn bilinear(map: &[f32], sw: usize, sh: usize, f: usize, x: usize, y: usize) -> f32 {
    let coord = |p: usize, n: usize| {
        let c = ((p as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (n - 1) as f32);
        let i = (c as usize).min(n.saturating_sub(2));
        (i, (i + 1).min(n - 1), c - i as f32)
    };
    let (x0, x1, tx) = coord(x, sw);
    let (y0, y1, ty) = coord(y, sh);
    let at = |x, y| map[y * sw + x];
    let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
    let bot = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
    top * (1.0 - ty) + bot * ty
}

/// Low/high percentile of `v`, from a sample of at most ~250k values.
fn percentiles(v: &[f32]) -> (f32, f32) {
    let step = (v.len() / 250_000).max(1);
    let mut s: Vec<f32> = v.iter().step_by(step).copied().collect();
    s.sort_unstable_by(f32::total_cmp);
    let at = |p: f32| s[((s.len() - 1) as f32 * p).round() as usize];
    (at(LOW_PCT), at(HIGH_PCT))
}

/// Approximate Gaussian blur: three box blurs per axis. Cost doesn't depend
/// on sigma. Edges are clamped.
fn gaussian_blur(img: &mut [f32], width: usize, height: usize, sigma: f32) {
    let r = (((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as usize;
    let mut line = Vec::new();
    for row in img.chunks_exact_mut(width) {
        for _ in 0..3 {
            box_blur_line(row, r, &mut line);
        }
    }
    let mut col = vec![0f32; height];
    for x in 0..width {
        for (y, c) in col.iter_mut().enumerate() {
            *c = img[y * width + x];
        }
        for _ in 0..3 {
            box_blur_line(&mut col, r, &mut line);
        }
        for (y, &c) in col.iter().enumerate() {
            img[y * width + x] = c;
        }
    }
}

/// Box blur of radius `r` in place, with clamped edges (running sum).
fn box_blur_line(v: &mut [f32], r: usize, scratch: &mut Vec<f32>) {
    let n = v.len() as isize;
    let r = r as isize;
    let at = |i: isize| v[i.clamp(0, n - 1) as usize];
    scratch.clear();
    let mut sum: f32 = (-r..=r).map(at).sum();
    for i in 0..n {
        scratch.push(sum);
        sum += at(i + r + 1) - at(i - r);
    }
    let norm = 1.0 / (2 * r + 1) as f32;
    for (o, &s) in v.iter_mut().zip(scratch.iter()) {
        *o = s * norm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gray RGB image from per-pixel levels.
    fn gray(levels: &[u8]) -> Vec<u8> {
        levels.iter().flat_map(|&l| [l, l, l]).collect()
    }

    fn mean(px: &[u8]) -> f32 {
        px.iter().map(|&v| v as f32).sum::<f32>() / px.len() as f32
    }

    #[test]
    fn box_blur_keeps_constants_and_mass() {
        let mut v = vec![3.0; 10];
        box_blur_line(&mut v, 4, &mut Vec::new());
        assert!(v.iter().all(|&x| (x - 3.0).abs() < 1e-5));

        let mut v = vec![0.0; 21];
        v[10] = 21.0;
        box_blur_line(&mut v, 2, &mut Vec::new());
        assert!((v.iter().sum::<f32>() - 21.0).abs() < 1e-4);
        assert_eq!(v[7], 0.0);
        assert!((v[8] - 4.2).abs() < 1e-5);
    }

    #[test]
    fn downscale_and_bilinear_roundtrip_smooth_maps() {
        // A linear ramp survives downscale + bilinear upsample (away from edges).
        let (w, h) = (40, 20);
        let img: Vec<f32> = (0..w * h).map(|i| (i % w) as f32 + 2.0 * (i / w) as f32).collect();
        let (small, sw, sh) = downscale(&img, w, h, 4);
        assert_eq!((sw, sh), (10, 5));
        assert!((small[0] - (1.5 + 3.0)).abs() < 1e-5);
        for (x, y) in [(5, 5), (20, 10), (33, 14)] {
            let got = bilinear(&small, sw, sh, 4, x, y);
            assert!((got - img[y * w + x]).abs() < 1e-3, "({x},{y}): {got}");
        }
        // f = 1 is the identity.
        assert_eq!(bilinear(&img, w, h, 1, 7, 3), img[3 * w + 7]);
    }

    #[test]
    fn evens_out_illumination() {
        // The same texture lit dimly on the left, brightly on the right.
        let (w, h) = (200, 50);
        let levels: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let texture = if (x / 3 + y / 3) % 2 == 0 { 0.6 } else { 1.0 };
                let light = if x < w / 2 { 40.0 } else { 220.0 };
                (texture * light) as u8
            })
            .collect();
        let mut img = gray(&levels);
        let half_means = |img: &[u8]| {
            let (mut l, mut r) = (Vec::new(), Vec::new());
            for (i, px) in img.chunks(3).enumerate() {
                // Skip the seam, where the blur straddles both halves.
                match i % w {
                    x if x < w / 2 - 20 => l.push(px[0]),
                    x if x >= w / 2 + 20 => r.push(px[0]),
                    _ => {}
                }
            }
            (mean(&l), mean(&r))
        };
        let (l0, r0) = half_means(&img);
        apply(&mut img, w, h, w * 3, 3);
        let (l1, r1) = half_means(&img);
        assert!((r1 - l1).abs() < 0.2 * (r0 - l0), "{l0} {r0} -> {l1} {r1}");
    }

    #[test]
    fn keeps_hue_and_alpha_and_row_padding() {
        // 2x1 RGBA with a padded rowstride: reddish pixel, then a darker one.
        let mut data = vec![200, 100, 50, 7, 100, 50, 25, 9, 42, 42];
        apply(&mut data, 2, 1, 10, 4);
        assert_eq!((data[3], data[7]), (7, 9));
        assert_eq!(&data[8..], &[42, 42]);
        for px in [&data[0..3], &data[4..7]] {
            assert!(px[0] >= px[1] && px[1] >= px[2], "{px:?}");
        }
    }

    #[test]
    fn flat_and_empty_images_dont_panic() {
        let mut img = gray(&[80; 16]);
        apply(&mut img, 4, 4, 12, 3);
        let mut img = gray(&[80; 1200 * 2]);
        apply(&mut img, 1200, 2, 3600, 3);
        let mut img = gray(&[80; 1200 * 1100]);
        apply(&mut img, 1200, 1100, 3600, 3);
        apply(&mut [], 0, 0, 0, 3);
    }
}
