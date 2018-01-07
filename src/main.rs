extern crate racecar;

use std::env;
use std::fs::File;
use std::io::Read;
use std::path::Path;

fn main() {
    let arg = env::args().nth(1).unwrap();
    let path = Path::new(&arg);
    let mut file = File::open(path).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    let replay = racecar::get_replay(&bytes, 0).unwrap().1;
    println!("{:#?}", replay)
}
