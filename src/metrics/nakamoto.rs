/// Nakamoto coefficient: minimum number of entities whose combined share
/// is strictly greater than `threshold` of the total population.
///
/// `entity_sizes` is the cluster size (or weight) per inferred entity.
/// Returns `None` when the input is empty or sums to zero.
pub fn nakamoto_coefficient(entity_sizes: &[u64], threshold: f64) -> Option<u64> {
    let total: u64 = entity_sizes.iter().copied().sum();
    if total == 0 {
        return None;
    }
    let mut sizes: Vec<u64> = entity_sizes.to_vec();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let mut acc: u64 = 0;
    for (i, s) in sizes.iter().enumerate() {
        acc += s;
        if (acc as f64) / (total as f64) > threshold {
            return Some((i + 1) as u64);
        }
    }
    Some(sizes.len() as u64)
}

/// Gini coefficient over a non-negative size distribution. Returns a value
/// in `[0.0, 1.0)`. Returns `None` when the input is empty or sums to zero.
pub fn gini(entity_sizes: &[u64]) -> Option<f64> {
    let n = entity_sizes.len();
    if n == 0 {
        return None;
    }
    let total: u64 = entity_sizes.iter().copied().sum();
    if total == 0 {
        return None;
    }
    let mut sorted: Vec<u64> = entity_sizes.to_vec();
    sorted.sort_unstable();
    let mut cumulative: f64 = 0.0;
    for (i, x) in sorted.iter().enumerate() {
        cumulative += ((i as f64) + 1.0) * (*x as f64);
    }
    let n_f = n as f64;
    let total_f = total as f64;
    Some((2.0 * cumulative) / (n_f * total_f) - (n_f + 1.0) / n_f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nakamoto_basic() {
        // Two entities of size 5 each, threshold 0.5 -> need 2 to exceed.
        assert_eq!(nakamoto_coefficient(&[5, 5], 0.5), Some(2));
        // One dominant entity exceeds 0.5 by itself.
        assert_eq!(nakamoto_coefficient(&[10, 1, 1], 0.5), Some(1));
    }

    #[test]
    fn gini_uniform_is_zero() {
        let g = gini(&[3, 3, 3, 3]).unwrap();
        assert!(g.abs() < 1e-9);
    }

    #[test]
    fn gini_concentrated_is_high() {
        let g = gini(&[100, 0, 0, 0]).unwrap();
        assert!(g > 0.7);
    }
}
