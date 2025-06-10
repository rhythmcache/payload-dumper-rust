use anyhow::{Result, anyhow};
use bsdiff;
use std::io::Cursor;

/// Apply a bsdiff patch to old data to produce new data

pub fn bspatch(old_data: &[u8], patch_data: &[u8]) -> Result<Vec<u8>> {
    let mut new_data = Vec::new();
    let mut patch_cursor = Cursor::new(patch_data);

    bsdiff::patch(old_data, &mut patch_cursor, &mut new_data)
        .map_err(|e| anyhow!("bsdiff patch failed: {}", e))?;

    Ok(new_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bspatch_simple() {
        // Create some test data
        let old_data = vec![1, 2, 3, 4, 5];
        let new_data = vec![1, 2, 4, 6];

        // Create a patch
        let mut patch = Vec::new();
        bsdiff::diff(&old_data, &new_data, &mut patch).unwrap();

        // Apply the patch
        let result = bspatch(&old_data, &patch).unwrap();
        assert_eq!(result, new_data);
    }
}
