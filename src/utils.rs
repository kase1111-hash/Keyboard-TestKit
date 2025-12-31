//! Shared utility functions and traits

/// Extension trait for tracking minimum and maximum values in Option<T>.
///
/// Provides convenient methods to update optional min/max values without
/// verbose `map().unwrap_or()` patterns.
///
/// # Example
///
/// ```
/// use keyboard_testkit::utils::MinMaxExt;
///
/// let mut min: Option<u64> = None;
/// let mut max: Option<u64> = None;
///
/// min.update_min(50);
/// max.update_max(50);
/// assert_eq!(min, Some(50));
/// assert_eq!(max, Some(50));
///
/// min.update_min(30);
/// max.update_max(70);
/// assert_eq!(min, Some(30));
/// assert_eq!(max, Some(70));
///
/// min.update_min(40);  // 40 > 30, so min stays 30
/// max.update_max(60);  // 60 < 70, so max stays 70
/// assert_eq!(min, Some(30));
/// assert_eq!(max, Some(70));
/// ```
pub trait MinMaxExt<T: Ord + Copy> {
    /// Updates the minimum value, storing the new value if it's smaller
    /// than the current minimum or if no minimum exists yet.
    fn update_min(&mut self, value: T);

    /// Updates the maximum value, storing the new value if it's larger
    /// than the current maximum or if no maximum exists yet.
    fn update_max(&mut self, value: T);
}

impl<T: Ord + Copy> MinMaxExt<T> for Option<T> {
    fn update_min(&mut self, value: T) {
        *self = Some(self.map(|m| m.min(value)).unwrap_or(value));
    }

    fn update_max(&mut self, value: T) {
        *self = Some(self.map(|m| m.max(value)).unwrap_or(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_min_from_none() {
        let mut min: Option<u64> = None;
        min.update_min(100);
        assert_eq!(min, Some(100));
    }

    #[test]
    fn update_min_smaller_value() {
        let mut min: Option<u64> = Some(100);
        min.update_min(50);
        assert_eq!(min, Some(50));
    }

    #[test]
    fn update_min_larger_value_unchanged() {
        let mut min: Option<u64> = Some(50);
        min.update_min(100);
        assert_eq!(min, Some(50));
    }

    #[test]
    fn update_max_from_none() {
        let mut max: Option<u64> = None;
        max.update_max(100);
        assert_eq!(max, Some(100));
    }

    #[test]
    fn update_max_larger_value() {
        let mut max: Option<u64> = Some(100);
        max.update_max(200);
        assert_eq!(max, Some(200));
    }

    #[test]
    fn update_max_smaller_value_unchanged() {
        let mut max: Option<u64> = Some(200);
        max.update_max(100);
        assert_eq!(max, Some(200));
    }

    #[test]
    fn update_min_max_sequence() {
        let mut min: Option<i32> = None;
        let mut max: Option<i32> = None;

        for value in [50, 30, 70, 20, 80, 40] {
            min.update_min(value);
            max.update_max(value);
        }

        assert_eq!(min, Some(20));
        assert_eq!(max, Some(80));
    }

    #[test]
    fn works_with_f64() {
        // f64 doesn't implement Ord, but let's test with ordered types
        let mut min: Option<u32> = None;
        let mut max: Option<u32> = None;

        min.update_min(100);
        max.update_max(100);
        min.update_min(50);
        max.update_max(150);

        assert_eq!(min, Some(50));
        assert_eq!(max, Some(150));
    }
}
