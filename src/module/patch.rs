use anyhow::{Result, anyhow};
use byteorder::{BigEndian, ReadBytesExt};
use bzip2::read::BzDecoder;
use std::io::{Cursor, Read};

const BSDF2_MAGIC: &[u8] = b"BSDF2";

pub fn bsdf2_read_patch<R: Read>(reader: &mut R) -> Result<(i64, Vec<(i64, i64, i64)>, Vec<u8>, Vec<u8>)> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;

    let (alg_control, alg_diff, alg_extra) = if magic.starts_with(BSDF2_MAGIC) {
        (magic[5], magic[6], magic[7])
    } else {
        return Err(anyhow!("Incorrect BSDF2 magic header"));
    };
    
    let len_control = reader.read_i64::<BigEndian>()?;
    let len_diff = reader.read_i64::<BigEndian>()?;
    let len_dst = reader.read_i64::<BigEndian>()?;

    
    let mut control_data = vec![0u8; len_control as usize];
    reader.read_exact(&mut control_data)?;
    let control_data = decompress_data(control_data, alg_control)?;

    
    let mut control = Vec::new();
    let mut i = 0;
    while i + 24 <= control_data.len() {
        let diff_len = i64::from_be_bytes([
            control_data[i], control_data[i + 1], control_data[i + 2], control_data[i + 3],
            control_data[i + 4], control_data[i + 5], control_data[i + 6], control_data[i + 7],
        ]);
        let extra_len = i64::from_be_bytes([
            control_data[i + 8], control_data[i + 9], control_data[i + 10], control_data[i + 11],
            control_data[i + 12], control_data[i + 13], control_data[i + 14], control_data[i + 15],
        ]);
        let seek_adjustment = i64::from_be_bytes([
            control_data[i + 16], control_data[i + 17], control_data[i + 18], control_data[i + 19],
            control_data[i + 20], control_data[i + 21], control_data[i + 22], control_data[i + 23],
        ]);
        control.push((diff_len, extra_len, seek_adjustment));
        i += 24;
    }

    
    let mut diff_data = vec![0u8; len_diff as usize];
    reader.read_exact(&mut diff_data)?;
    let diff_data = decompress_data(diff_data, alg_diff)?;

    
    let mut extra_data = Vec::new();
    let mut remaining_data = Vec::new();
    if reader.read_to_end(&mut remaining_data)? > 0 {
        extra_data = decompress_data(remaining_data, alg_extra)?;
    }

    Ok((len_dst, control, diff_data, extra_data))
}

fn decompress_data(data: Vec<u8>, algorithm: u8) -> Result<Vec<u8>> {
    match algorithm {
        0 => Ok(data),
        1 => {
            let mut decoder = BzDecoder::new(&data[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            Ok(decompressed)
        }
        2 => {
            
            let mut decompressed = Vec::new();
            brotli::Decompressor::new(&data[..], 4096).read_to_end(&mut decompressed)?;
            Ok(decompressed)
        }
        3 => {
            
            zstd::decode_all(Cursor::new(&data)).map_err(|e| anyhow!("Zstd decompression failed: {}", e))
        }
        _ => Err(anyhow!("Unsupported compression algorithm: {}", algorithm)),
    }
}

pub fn bspatch(old_data: &[u8], patch_data: &[u8]) -> Result<Vec<u8>> {
    let mut reader = Cursor::new(patch_data);
    let (len_dst, control, diff_data, extra_data) = bsdf2_read_patch(&mut reader)?;

    let mut new_data = vec![0u8; len_dst as usize];
    let mut old_pos: i64 = 0;
    let mut new_pos: usize = 0;
    let mut diff_pos: usize = 0;
    let mut extra_pos: usize = 0;

    for (diff_len, extra_len, seek_adjustment) in control {
        if diff_len < 0 || extra_len < 0 {
            return Err(anyhow!("Invalid control data: negative lengths"));
        }
        
        let diff_len = diff_len as usize;
        let extra_len = extra_len as usize;

        
        if new_pos + diff_len > new_data.len() {
            return Err(anyhow!("bspatch: diff operation exceeds output size"));
        }
        
        for i in 0..diff_len {
            let old_byte = if (old_pos as usize + i) < old_data.len() {
                old_data[old_pos as usize + i]
            } else {
                0
            };
            
            let diff_byte = if diff_pos + i < diff_data.len() {
                diff_data[diff_pos + i]
            } else {
                0
            };
            
            new_data[new_pos + i] = old_byte.wrapping_add(diff_byte);
        }
        
        new_pos += diff_len;
        old_pos += diff_len as i64;
        diff_pos += diff_len;

        
        if new_pos + extra_len > new_data.len() {
            return Err(anyhow!("bspatch: extra copy exceeds output size"));
        }
        
        for i in 0..extra_len {
            if extra_pos + i < extra_data.len() {
                new_data[new_pos + i] = extra_data[extra_pos + i];
            } else {
                new_data[new_pos + i] = 0;
            }
        }
        
        new_pos += extra_len;
        extra_pos += extra_len;

        
        old_pos += seek_adjustment;
        
        if old_pos < 0 {
            return Err(anyhow!("bspatch: seek made old_pos negative"));
        }
    }

    Ok(new_data)
}
