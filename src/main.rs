mod module;
use std::io::{Read, Seek};

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}
include!("proto/update_metadata.rs");

fn main() {
    if let Err(e) = module::entry::run() {
        eprintln!("- Error: {}", e);
        std::process::exit(1);
    }
}
