//! Auto-brightness: carry the *displayed* brightness of one image over to the
//! next. Images are summarised as a 256-bin luminance histogram; the viewer's
//! adjustment is `out = clamp(in^(1/gamma) * brightness, 0, 1)` per channel.

/// Brightness range the viewer allows.
pub const MIN_BRIGHTNESS: f64 = 0.1;
pub const MAX_BRIGHTNESS: f64 = 5.0;

pub type Histogram = [u64; 256];

/// Luminance histogram of packed 8-bit RGB(A) pixels. `step` samples every
/// n-th pixel (per row) to bound the cost on huge images.
pub fn luminance_histogram(
    data: &[u8],
    width: usize,
    height: usize,
    rowstride: usize,
    n_channels: usize,
    step: usize,
) -> Histogram {
    let mut hist = [0u64; 256];
    let step = step.max(1);
    for y in (0..height).step_by(step) {
        let row = &data[y * rowstride..y * rowstride + width * n_channels];
        for px in row.chunks_exact(n_channels).step_by(step) {
            // Rec. 601 luma in fixed point.
            let l = (299 * px[0] as u32 + 587 * px[1] as u32 + 114 * px[2] as u32 + 500) / 1000;
            hist[l.min(255) as usize] += 1;
        }
    }
    hist
}

/// Mean displayed luminance (0..1) of an image with this histogram under the
/// given brightness/gamma. `None` for an empty histogram.
pub fn displayed_mean(hist: &Histogram, brightness: f64, gamma: f64) -> Option<f64> {
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return None;
    }
    let inv_g = 1.0 / gamma.max(0.05);
    let sum: f64 = hist
        .iter()
        .enumerate()
        .filter(|(_, &c)| c > 0)
        .map(|(i, &c)| c as f64 * ((i as f64 / 255.0).powf(inv_g) * brightness).min(1.0))
        .sum();
    Some(sum / total as f64)
}

/// Brightness that makes an image with `hist` display at mean luminance
/// `target` under `gamma`, clamped to the viewer's range. The displayed mean
/// is monotonic in brightness, so bisect.
pub fn match_brightness(hist: &Histogram, gamma: f64, target: f64) -> Option<f64> {
    let mean_at = |b| displayed_mean(hist, b, gamma);
    if mean_at(MAX_BRIGHTNESS)? <= target {
        return Some(MAX_BRIGHTNESS);
    }
    if mean_at(MIN_BRIGHTNESS)? >= target {
        return Some(MIN_BRIGHTNESS);
    }
    let (mut lo, mut hi) = (MIN_BRIGHTNESS, MAX_BRIGHTNESS);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if mean_at(mid)? < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(level: usize) -> Histogram {
        let mut h = [0u64; 256];
        h[level] = 100;
        h
    }

    #[test]
    fn empty_histogram_has_no_mean() {
        assert_eq!(displayed_mean(&[0; 256], 1.0, 1.0), None);
        assert_eq!(match_brightness(&[0; 256], 1.0, 0.5), None);
    }

    #[test]
    fn identity_mean() {
        let m = displayed_mean(&flat(51), 1.0, 1.0).unwrap();
        assert!((m - 0.2).abs() < 1e-9);
    }

    #[test]
    fn darker_image_gets_boosted_to_match() {
        // Previous image: level 128 shown at 1.0. Next image is half as bright.
        let target = displayed_mean(&flat(128), 1.0, 1.0).unwrap();
        let b = match_brightness(&flat(64), 1.0, target).unwrap();
        assert!((b - 2.0).abs() < 1e-3, "{b}");
    }

    #[test]
    fn match_respects_gamma() {
        let target = 0.5;
        let b = match_brightness(&flat(64), 2.0, target).unwrap();
        let got = displayed_mean(&flat(64), b, 2.0).unwrap();
        assert!((got - target).abs() < 1e-6);
    }

    #[test]
    fn clipping_is_accounted_for() {
        // Half black, half near-white: boosting clips the bright half, so the
        // needed brightness is higher than the naive ratio.
        let mut h = [0u64; 256];
        h[0] = 50;
        h[200] = 50;
        let target = 0.45;
        let b = match_brightness(&h, 1.0, target).unwrap();
        assert!((displayed_mean(&h, b, 1.0).unwrap() - target).abs() < 1e-6);
    }

    #[test]
    fn unreachable_targets_clamp() {
        assert_eq!(match_brightness(&flat(0), 1.0, 0.5), Some(MAX_BRIGHTNESS));
        assert_eq!(match_brightness(&flat(255), 1.0, 0.01), Some(MIN_BRIGHTNESS));
    }

    #[test]
    fn histogram_counts_luma_with_sampling() {
        // 2x2 RGB image, rowstride padded to 8: white, black / red, gray.
        let data = [255, 255, 255, 0, 0, 0, 9, 9, 255, 0, 0, 128, 128, 128, 9, 9];
        let h = luminance_histogram(&data, 2, 2, 8, 3, 1);
        assert_eq!(h[255], 1);
        assert_eq!(h[0], 1);
        assert_eq!(h[76], 1); // 0.299 * 255
        assert_eq!(h[128], 1);
        let h = luminance_histogram(&data, 2, 2, 8, 3, 2);
        assert_eq!(h.iter().sum::<u64>(), 1);
    }
}
