//! Channel count conversion at the boundary between a device and the graph.

/// Converts interleaved `src` (`src_ch` channels) to `dst` (`dst_ch` channels), frame by frame, over
/// `min(frames)` frames. Returns the number of frames written.
///
/// * mono -> n: the sample is copied to the first two channels (front left/right), others silent;
/// * n -> mono: average of the first two channels (or the only one);
/// * stereo -> stereo: copy;
/// * more channels -> fewer: the first channels are kept (front left/right first), extra ones dropped;
/// * fewer -> more: existing channels kept, the rest silent.
pub fn remix(src: &[f32], src_ch: usize, dst: &mut [f32], dst_ch: usize) -> usize {
    if src_ch == 0 || dst_ch == 0 {
        return 0;
    }
    let frames = (src.len() / src_ch).min(dst.len() / dst_ch);
    for f in 0..frames {
        let s = &src[f * src_ch..(f + 1) * src_ch];
        let d = &mut dst[f * dst_ch..(f + 1) * dst_ch];
        match (src_ch, dst_ch) {
            (a, b) if a == b => d.copy_from_slice(s),
            (1, _) => {
                d.fill(0.0);
                d[0] = s[0];
                if dst_ch > 1 {
                    d[1] = s[0];
                }
            }
            (_, 1) => {
                d[0] = if src_ch >= 2 {
                    0.5 * (s[0] + s[1])
                } else {
                    s[0]
                }
            }
            _ => {
                d.fill(0.0);
                let n = src_ch.min(dst_ch);
                d[..n].copy_from_slice(&s[..n]);
            }
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut out = [9.0; 6];
        assert_eq!(remix(&[1.0, 2.0, 3.0], 1, &mut out, 2), 3);
        assert_eq!(out, [1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let mut out = [0.0; 2];
        remix(&[1.0, 3.0, -1.0, 1.0], 2, &mut out, 1);
        assert_eq!(out, [2.0, 0.0]);
    }

    #[test]
    fn same_layout_copies() {
        let mut out = [0.0; 4];
        remix(&[1.0, 2.0, 3.0, 4.0], 2, &mut out, 2);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn surround_to_stereo_keeps_front_pair() {
        let mut out = [0.0; 2];
        remix(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 6, &mut out, 2);
        assert_eq!(out, [1.0, 2.0]);
    }

    #[test]
    fn stereo_to_surround_fills_front_pair_only() {
        let mut out = [9.0; 6];
        remix(&[1.0, 2.0], 2, &mut out, 6);
        assert_eq!(out, [1.0, 2.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn stops_at_the_shorter_side() {
        let mut out = [0.0; 4];
        assert_eq!(remix(&[1.0; 20], 2, &mut out, 2), 2);
    }
}
