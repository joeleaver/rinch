use super::*;

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

mod comparison;
mod conversion;
mod fixed_point;
mod parsing;
mod tokens;
