use anyhow::{Result, anyhow};
use byteorder::{BigEndian, ReadBytesExt};
use bzip2::read::BzDecoder;
use std::io::{Cursor, Read};

const BSDF2_MAGIC: &[u8] = b"BSDF2";

pub fn bsdf2_read_patch<R: Read>(reader: &mut R) -> Result<(i64, Vec<(i64, i64, i64)>, Vec<u8>)> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;

    let (alg_control, alg_diff, _) = if magic.starts_with(BSDF2_MAGIC) {
        (magic[5], magic[6], magic[7])
    } else {
        return Err(anyhow!("Incorrect BSDF2 magic header"));
    };
    let len_control = reader.read_i64::<BigEndian>()?;
    let len_diff = reader.read_i64::<BigEndian>()?;
    let len_dst = reader.read_i64::<BigEndian>()?;

    let mut control_data = vec![0u8; len_control as usize];
    reader.read_exact(&mut control_data)?;
    let control_data = match alg_control {
        0 => control_data,
        1 => {
            let mut decoder = BzDecoder::new(&control_data[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            decompressed
        }
        2 => {
            let mut decompressed = Vec::new();
            let decompressed_size = brotli::Decompressor::new(&control_data[..], 4096)
                .read_to_end(&mut decompressed)?;
            decompressed.truncate(decompressed_size);
            decompressed
        }
        3 => match zstd::decode_all(Cursor::new(&control_data)) {
            Ok(decompressed) => decompressed,
            Err(e) => {
                return Err(anyhow!(
                    "Failed to decompress control data with Zstd: {}",
                    e
                ));
            }
        },
        _ => {
            return Err(anyhow!(
                "Unsupported control compression algorithm: {}",
                alg_control
            ));
        }
    };
    let mut control = Vec::new();
    let mut i = 0;
    while i < control_data.len() {
        if i + 24 > control_data.len() {
            break;
        }
        let x = i64::from_be_bytes([
            control_data[i],
            control_data[i + 1],
            control_data[i + 2],
            control_data[i + 3],
            control_data[i + 4],
            control_data[i + 5],
            control_data[i + 6],
            control_data[i + 7],
        ]);
        let y = i64::from_be_bytes([
            control_data[i + 8],
            control_data[i + 9],
            control_data[i + 10],
            control_data[i + 11],
            control_data[i + 12],
            control_data[i + 13],
            control_data[i + 14],
            control_data[i + 15],
        ]);
        let z = i64::from_be_bytes([
            control_data[i + 16],
            control_data[i + 17],
            control_data[i + 18],
            control_data[i + 19],
            control_data[i + 20],
            control_data[i + 21],
            control_data[i + 22],
            control_data[i + 23],
        ]);
        control.push((x, y, z));
        i += 24;
    }
    let mut diff_data = vec![0u8; len_diff as usize];
    reader.read_exact(&mut diff_data)?;
    let diff_data = match alg_diff {
        0 => diff_data,
        1 => {
            let mut decoder = BzDecoder::new(&diff_data[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            decompressed
        }
        2 => {
            let mut decompressed = Vec::new();
            let decompressed_size =
                brotli::Decompressor::new(&diff_data[..], 4096).read_to_end(&mut decompressed)?;
            decompressed.truncate(decompressed_size);
            decompressed
        }
        _ => {
            return Err(anyhow!(
                "Unsupported diff compression algorithm: {}",
                alg_diff
            ));
        }
    };
    Ok((len_dst, control, diff_data))
}
pub fn bspatch(old_data: &[u8], patch_data: &[u8]) -> Result<Vec<u8>> {
    let mut reader = Cursor::new(patch_data);
    let (len_dst, control, diff_data) = bsdf2_read_patch(&mut reader)?;

    let mut new_data = vec![0u8; len_dst as usize];
    let mut old_pos = 0;
    let mut new_pos = 0;

    for (diff_len, _, seek_adjustment) in control {
        if new_pos + diff_len as usize > new_data.len()
            || old_pos + diff_len as usize > old_data.len()
        {
            return Err(anyhow!("Invalid bspatch control data"));
        }

        for i in 0..diff_len as usize {
            if old_pos + i < old_data.len() && new_pos + i < new_data.len() && i < diff_data.len() {
                new_data[new_pos + i] = old_data[old_pos + i].wrapping_add(diff_data[i]);
            }
        }
        new_pos += diff_len as usize;
        old_pos += diff_len as usize;

        old_pos = (old_pos as i64 + seek_adjustment) as usize;
    }

    Ok(new_data)
}
