mod module;
use std::io::{Read, Seek};

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}
include!("proto/update_metadata.rs");
const BSDF2_MAGIC: &[u8] = b"BSDF2";

fn main() {
    let _ = module::entry::run();
}
