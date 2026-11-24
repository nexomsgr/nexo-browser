//! Масштаб страницы: те же ступени, что в привычных браузерах (25 % … 500 %).

pub const STEPS: [f64; 17] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0,
];

const EPS: f64 = 1e-6;

/// Следующая ступень вверх или вниз от текущего масштаба (текущий может не лежать на ступени).
pub fn step(current: f64, up: bool) -> f64 {
    let cur = if current.is_finite() { current } else { 1.0 };
    if up {
        STEPS
            .iter()
            .copied()
            .find(|s| *s > cur + EPS)
            .unwrap_or(STEPS[STEPS.len() - 1])
    } else {
        STEPS
            .iter()
            .rev()
            .copied()
            .find(|s| *s < cur - EPS)
            .unwrap_or(STEPS[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_up_and_down_from_100() {
        assert_eq!(step(1.0, true), 1.1);
        assert_eq!(step(1.0, false), 0.9);
    }

    #[test]
    fn clamps_at_the_ends() {
        assert_eq!(step(5.0, true), 5.0);
        assert_eq!(step(0.25, false), 0.25);
    }

    #[test]
    fn off_grid_values_move_to_the_nearest_step_in_that_direction() {
        assert_eq!(step(1.05, true), 1.1);
        assert_eq!(step(1.05, false), 1.0);
    }

    #[test]
    fn garbage_is_treated_as_100_percent() {
        assert_eq!(step(f64::NAN, true), 1.1);
    }

    #[test]
    fn steps_are_sorted() {
        assert!(STEPS.windows(2).all(|w| w[0] < w[1]));
    }
}
