fn main() { let n = 20000000usize; let mut p = vec![true; n + 1]; p[0] = false; p[1] = false;
 let mut i = 2; while i * i <= n { if p[i] { let mut j = i * i; while j <= n { p[j] = false; j += i } } i += 1 }
 println!("{}", p.iter().filter(|&&b| b).count()); }
