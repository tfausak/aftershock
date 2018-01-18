extern crate aftershock;

use std::env;
use std::fs::File;
use std::io::Read;
use std::time::Instant;

fn main() {
    let mut bytes = Vec::new();
    File::open(env::args().nth(1).unwrap())
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let size = bytes.len();
    let size_mb = usize_f64(size) / 1_048_576.;

    let start = Instant::now();
    let result = aftershock::Get::new(bytes).get_replay();
    let elapsed = start.elapsed();
    let elapsed_ms =
        u64_f64(1_000_000_000 * elapsed.as_secs() + u32_u64(elapsed.subsec_nanos())) / 1_000_000.;

    let rate = 1_000. * size_mb / elapsed_ms;
    eprintln!(
        "Parsed {:.3} MB in {:.3} ms at {:.3} MB/s.",
        size_mb, elapsed_ms, rate
    );

    match result {
        Err(problem) => eprintln!("{:#?}", problem),
        Ok(replay) => println!("{:#?}", replay),
    }
}

fn u32_u64(x: u32) -> u64 {
    u64::from(x)
}

fn u64_f64(x: u64) -> f64 {
    x as f64
}

fn usize_f64(x: usize) -> f64 {
    x as f64
}
