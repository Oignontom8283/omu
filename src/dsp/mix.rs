//! Summing and metering.

/// `dst[i] += src[i]` over the common length.
pub fn add_into(dst: &mut [f32], src: &[f32]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d += *s;
    }
}

/// Largest absolute sample value (linear, 1.0 = full scale).
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_into_sums_and_stops_at_the_shorter() {
        let mut a = vec![1.0, 2.0, 3.0];
        add_into(&mut a, &[10.0, 20.0]);
        assert_eq!(a, vec![11.0, 22.0, 3.0]);
    }

    #[test]
    fn peak_is_the_largest_magnitude() {
        assert_eq!(peak(&[0.1, -0.7, 0.3]), 0.7);
        assert_eq!(peak(&[]), 0.0);
    }
}
