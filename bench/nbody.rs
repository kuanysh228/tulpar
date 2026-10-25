fn f(x: f64) -> f64 { x * x * 0.5 + 1.0 / (x + 1.0) }
fn main() { let mut s = 0.0f64; for i in 0..100000000i64 { s += f(i as f64 * 0.000001) } println!("{:.4}", s); }
