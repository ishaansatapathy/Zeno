//! How a file of a given size is cut into upload parts. The plan is a pure function of the
//! size, so the exact length of every part is known before the upload starts and can be
//! signed into its URL.

pub const MIB: u64 = 1024 * 1024;
/// S3 rejects any part but the last below this.
pub const MIN_PART: u64 = 5 * MIB;
const TARGET_PART: u64 = 16 * MIB;
/// Caps how many URLs one response carries; well under S3's own limit of 10,000.
const MAX_PARTS: u64 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartPlan {
    pub total_size: u64,
    pub part_size: u64,
    pub count: u16,
}

impl PartPlan {
    pub fn new(total_size: u64) -> Self {
        let wanted = total_size.div_ceil(MAX_PARTS).div_ceil(MIB) * MIB;
        Self::from_parts(total_size, TARGET_PART.max(wanted))
    }

    /// Rebuilds a plan from stored values.
    pub fn from_parts(total_size: u64, part_size: u64) -> Self {
        let count = total_size.div_ceil(part_size).max(1);
        Self {
            total_size,
            part_size,
            count: u16::try_from(count).expect("part count fits in u16"),
        }
    }

    /// Exact byte length of part `number` (1-based), or `None` if there is no such part.
    pub fn size_of(&self, number: u16) -> Option<u64> {
        if number == 0 || number > self.count {
            return None;
        }
        if number < self.count {
            Some(self.part_size)
        } else {
            Some(self.total_size - self.part_size * u64::from(self.count - 1))
        }
    }

    pub fn numbers(&self) -> impl Iterator<Item = u16> {
        1..=self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_files_are_one_part_of_exactly_their_size() {
        for size in [1, 1234, MIB, 16 * MIB] {
            let plan = PartPlan::new(size);
            assert_eq!((plan.count, plan.size_of(1)), (1, Some(size)), "{size}");
        }
    }

    #[test]
    fn one_byte_over_the_target_adds_a_one_byte_final_part() {
        let plan = PartPlan::new(16 * MIB + 1);
        assert_eq!(plan.count, 2);
        assert_eq!(
            (plan.size_of(1), plan.size_of(2)),
            (Some(16 * MIB), Some(1))
        );
    }

    #[test]
    fn big_files_use_larger_parts_so_the_url_list_stays_bounded() {
        let plan = PartPlan::new(10 * 1024 * MIB);
        assert_eq!(plan.part_size, 40 * MIB);
        assert_eq!(plan.count, 256);
    }

    #[test]
    fn part_sizes_always_add_up_and_respect_the_s3_minimum() {
        let mut sizes: Vec<u64> = vec![
            1,
            5 * MIB,
            16 * MIB,
            17 * MIB,
            4096 * MIB + 7,
            10 * 1024 * MIB,
        ];
        let mut x: u64 = 0x9e3779b97f4a7c15;
        for _ in 0..500 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            sizes.push(1 + x % (10 * 1024 * MIB));
        }
        for size in sizes {
            let plan = PartPlan::new(size);
            assert!(
                plan.count >= 1 && u64::from(plan.count) <= MAX_PARTS,
                "{size}"
            );
            let sum: u64 = plan.numbers().map(|n| plan.size_of(n).unwrap()).sum();
            assert_eq!(sum, size, "parts must add up to the file for {size}");
            for n in plan.numbers() {
                let part = plan.size_of(n).unwrap();
                assert!(part >= 1);
                if n < plan.count {
                    assert!(
                        part >= MIN_PART,
                        "non-final part below the S3 minimum for {size}"
                    );
                }
            }
            assert_eq!(PartPlan::from_parts(size, plan.part_size), plan);
        }
    }

    #[test]
    fn there_is_no_part_zero_and_no_part_past_the_end() {
        let plan = PartPlan::new(40 * MIB);
        assert_eq!(plan.size_of(0), None);
        assert_eq!(plan.size_of(plan.count + 1), None);
    }
}
