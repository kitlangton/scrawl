//! Bounded visual measurements shared by retained-material consumers.
//!
//! These functions expose raw pixel measurements only. They do not select
//! frames, identify transitions, compare regions, or assign timestamps.

const THUMBNAIL_WIDTH: usize = 64;
const THUMBNAIL_HEIGHT: usize = 36;
const GRID_LIMIT: usize = 128;
const APOBS001_HEADER_BYTES: usize = 20;
pub const APOBS001_MAX_BYTES: usize = APOBS001_HEADER_BYTES + 4 * (64 * 36 + 128 * 128);

/// Samples the centers of a fixed 64×36 grid using the legacy keyframe math.
///
/// RGB is interpreted as encoded 8-bit color and converted with the legacy
/// Rec. 709 weights. Alpha is ignored. Missing sampled RGB bytes leave that
/// cell at zero, matching the former caller for partial buffers.
pub fn luma_thumbnail64x36(rgba: &[u8], width: u32, height: u32) -> [f32; 64 * 36] {
    let mut thumbnail = [0.0; THUMBNAIL_WIDTH * THUMBNAIL_HEIGHT];
    if width == 0 || height == 0 {
        return thumbnail;
    }
    let width = width as usize;
    let height = height as usize;
    for y in 0..THUMBNAIL_HEIGHT {
        let sy = ((2 * y + 1) * height / (2 * THUMBNAIL_HEIGHT)).min(height - 1);
        for x in 0..THUMBNAIL_WIDTH {
            let sx = ((2 * x + 1) * width / (2 * THUMBNAIL_WIDTH)).min(width - 1);
            let offset = (sy * width + sx) * 4;
            if let Some(pixel) = rgba.get(offset..offset + 3) {
                thumbnail[y * THUMBNAIL_WIDTH + x] = (0.2126 * pixel[0] as f32
                    + 0.7152 * pixel[1] as f32
                    + 0.0722 * pixel[2] as f32)
                    / 255.0;
            }
        }
    }
    thumbnail
}

/// Mean encoded-RGB Rec. 709 luma sampled into at most 128×128 cells.
///
/// Large inputs retain the legacy integer step `(max(width, height) / 512).max(1)`.
/// Per-sample luma is rounded down in 1/256ths before each cell's normalized
/// `f32` mean is calculated. The input may be short; incomplete rows and
/// unsampled cells remain zero as in the former keyframe caller.
pub fn luma_grid_up_to_128(rgba: &[u8], width: u32, height: u32) -> LumaGrid {
    let (width, height) = (width as usize, height as usize);
    let (columns, rows) = (width.min(GRID_LIMIT), height.min(GRID_LIMIT));
    let step = (width.max(height) / (GRID_LIMIT * 4)).max(1);
    let cell_x: Vec<usize> = (0..width)
        .step_by(step)
        .map(|x| x * columns / width)
        .collect();
    let mut sums = vec![0_u32; columns * rows];
    let mut counts = vec![0_u32; columns * rows];
    let lines = rgba.chunks_exact(width.max(1) * 4).take(height);
    for (y, line) in lines.enumerate().step_by(step) {
        let row = y * rows / height * columns;
        for (pixel, &x) in line.chunks_exact(4).step_by(step).zip(&cell_x) {
            sums[row + x] +=
                (54 * pixel[0] as u32 + 183 * pixel[1] as u32 + 19 * pixel[2] as u32) >> 8;
            counts[row + x] += 1;
        }
    }
    let cells = sums
        .iter()
        .zip(&counts)
        .map(|(&sum, &count)| sum as f32 / 255.0 / count.max(1) as f32)
        .collect();
    LumaGrid {
        columns,
        rows,
        cells,
    }
}

/// Raw dimensions and normalized luma means for a bounded measurement grid.
#[derive(Clone, Debug, PartialEq)]
pub struct LumaGrid {
    columns: usize,
    rows: usize,
    cells: Vec<f32>,
}

impl LumaGrid {
    pub const fn columns(&self) -> usize {
        self.columns
    }

    pub const fn rows(&self) -> usize {
        self.rows
    }

    pub fn cells(&self) -> &[f32] {
        &self.cells
    }
}

/// Serializes the bounded private `apOB` observation profile for one final
/// RGBA8 frame plane. Invalid or incomplete planes are rejected.
///
/// The profile is `APOBS001`, little-endian width/height, little-endian
/// region columns/rows, followed by thumbnail and row-major region luma as
/// little-endian `f32`. It contains measurements only; no selection policy.
pub fn apobs001_profile(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let expected_bytes = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if rgba.len() != expected_bytes {
        return None;
    }

    let thumbnail = luma_thumbnail64x36(rgba, width, height);
    let grid = luma_grid_up_to_128(rgba, width, height);
    let columns = u16::try_from(grid.columns()).ok()?;
    let rows = u16::try_from(grid.rows()).ok()?;
    let float_count = thumbnail.len().checked_add(grid.cells().len())?;
    let byte_len = APOBS001_HEADER_BYTES.checked_add(float_count.checked_mul(4)?)?;
    if byte_len > APOBS001_MAX_BYTES {
        return None;
    }

    let mut profile = Vec::with_capacity(byte_len);
    profile.extend_from_slice(b"APOBS001");
    profile.extend_from_slice(&width.to_le_bytes());
    profile.extend_from_slice(&height.to_le_bytes());
    profile.extend_from_slice(&columns.to_le_bytes());
    profile.extend_from_slice(&rows.to_le_bytes());
    for value in thumbnail.iter().chain(grid.cells()) {
        profile.extend_from_slice(&value.to_le_bytes());
    }
    Some(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_pixel(rgba: &mut [u8], width: usize, x: usize, y: usize, color: [u8; 4]) {
        let offset = (y * width + x) * 4;
        rgba[offset..offset + 4].copy_from_slice(&color);
    }

    fn profile_u16(profile: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(profile[offset..offset + 2].try_into().unwrap())
    }

    fn profile_u32(profile: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(profile[offset..offset + 4].try_into().unwrap())
    }

    fn profile_f32(profile: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(profile[offset..offset + 4].try_into().unwrap())
    }

    fn assert_profile_header(profile: &[u8], width: u32, height: u32, columns: u16, rows: u16) {
        assert_eq!(&profile[..8], b"APOBS001");
        assert_eq!(profile_u32(profile, 8), width);
        assert_eq!(profile_u32(profile, 12), height);
        assert_eq!(profile_u16(profile, 16), columns);
        assert_eq!(profile_u16(profile, 18), rows);
    }

    #[test]
    fn thumbnail_samples_cell_centers_with_legacy_rec709_float_math() {
        let (width, height) = (128, 72);
        let mut rgba = vec![0; width * height * 4];
        set_pixel(&mut rgba, width, 1, 1, [255, 0, 0, 0]);
        set_pixel(&mut rgba, width, 2, 1, [255, 255, 255, 255]);
        set_pixel(&mut rgba, width, 3, 1, [0, 255, 0, 255]);

        let thumbnail = luma_thumbnail64x36(&rgba, width as u32, height as u32);
        assert_eq!(thumbnail[0], (0.2126 * 255.0_f32) / 255.0);
        assert_eq!(thumbnail[1], (0.7152 * 255.0_f32) / 255.0);
        assert_eq!(thumbnail[2], 0.0, "even x coordinates are not cell centers");
        assert_eq!(thumbnail[64], 0.0, "the next row samples y=3, not y=1");
    }

    #[test]
    fn thumbnail_preserves_zero_and_partial_rgb_input_behavior() {
        assert!(
            luma_thumbnail64x36(&[], 0, 1)
                .iter()
                .all(|value| *value == 0.0)
        );
        assert!(
            luma_thumbnail64x36(&[], 1, 0)
                .iter()
                .all(|value| *value == 0.0)
        );
        assert!(
            luma_thumbnail64x36(&[], 1, 1)
                .iter()
                .all(|value| *value == 0.0)
        );

        let partial = luma_thumbnail64x36(&[255, 0, 0], 1, 1);
        assert!(
            partial
                .iter()
                .all(|value| *value == (0.2126 * 255.0_f32) / 255.0)
        );
    }

    #[test]
    fn grid_uses_integer_rec709_rounding_and_normalized_cell_mean() {
        let (width, height) = (256, 256);
        let mut rgba = vec![0; width * height * 4];
        set_pixel(&mut rgba, width, 0, 0, [255, 0, 0, 255]);
        let grid = luma_grid_up_to_128(&rgba, width as u32, height as u32);
        assert_eq!((grid.columns(), grid.rows()), (128, 128));
        assert_eq!(grid.cells()[0], 53.0_f32 / 255.0 / 4.0);

        let green = luma_grid_up_to_128(&[0, 255, 0, 255], 1, 1);
        assert_eq!(green.cells(), &[182.0_f32 / 255.0]);
        let blue = luma_grid_up_to_128(&[0, 0, 255, 255], 1, 1);
        assert_eq!(blue.cells(), &[18.0_f32 / 255.0]);
    }

    #[test]
    fn grid_keeps_legacy_step_for_thin_lines_and_clamps_dimensions() {
        let (width, height) = (1024, 1024);
        let mut sampled_line = vec![0; width * height * 4];
        for y in 0..height {
            set_pixel(&mut sampled_line, width, 6, y, [255, 255, 255, 255]);
        }
        let grid = luma_grid_up_to_128(&sampled_line, width as u32, height as u32);
        assert_eq!((grid.columns(), grid.rows()), (128, 128));
        assert_eq!(grid.cells()[0], 0.25, "step=2 samples x=6 once per cell");

        let mut skipped_line = vec![0; width * height * 4];
        for y in 0..height {
            set_pixel(&mut skipped_line, width, 7, y, [255, 255, 255, 255]);
        }
        assert_eq!(
            luma_grid_up_to_128(&skipped_line, width as u32, height as u32).cells()[0],
            0.0,
            "step=2 does not sample x=7"
        );

        let oversized = luma_grid_up_to_128(&[], 2049, 1025);
        assert_eq!((oversized.columns(), oversized.rows()), (128, 128));
        assert!(oversized.cells().iter().all(|value| *value == 0.0));
    }

    #[test]
    fn grid_preserves_zero_dimensions_and_incomplete_rows() {
        let zero_width = luma_grid_up_to_128(&[255, 0, 0, 255], 0, 4);
        assert_eq!((zero_width.columns(), zero_width.rows()), (0, 4));
        assert!(zero_width.cells().is_empty());

        let zero_height = luma_grid_up_to_128(&[255, 0, 0, 255], 4, 0);
        assert_eq!((zero_height.columns(), zero_height.rows()), (4, 0));
        assert!(zero_height.cells().is_empty());

        let incomplete = luma_grid_up_to_128(&[255, 0, 0, 255, 0, 0, 0, 255], 2, 2);
        assert_eq!((incomplete.columns(), incomplete.rows()), (2, 2));
        assert_eq!(incomplete.cells(), &[53.0_f32 / 255.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn apobs001_small_plane_matches_source_derived_luma_golden() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 255];
        let profile = apobs001_profile(&rgba, 2, 1).unwrap();
        assert_eq!(profile.len(), 20 + 4 * (64 * 36 + 2));
        assert_profile_header(&profile, 2, 1, 2, 1);

        let thumbnail_offset = APOBS001_HEADER_BYTES;
        let red = (0.2126 * 255.0_f32) / 255.0;
        let green = (0.7152 * 255.0_f32) / 255.0;
        for y in 0..36 {
            for x in 0..64 {
                let expected = if x < 32 { red } else { green };
                let offset = thumbnail_offset + (y * 64 + x) * 4;
                assert_eq!(profile_f32(&profile, offset), expected);
            }
        }
        let grid_offset = thumbnail_offset + 64 * 36 * 4;
        assert_eq!(profile_f32(&profile, grid_offset), 53.0_f32 / 255.0);
        assert_eq!(profile_f32(&profile, grid_offset + 4), 182.0_f32 / 255.0);
    }

    #[test]
    fn apobs001_odd_plane_matches_source_derived_luma_golden() {
        let (width, height) = (5, 3);
        let mut rgba = vec![0; width * height * 4];
        set_pixel(&mut rgba, width, 2, 1, [0, 0, 255, 255]);
        set_pixel(&mut rgba, width, 4, 2, [0, 255, 0, 255]);
        let profile = apobs001_profile(&rgba, width as u32, height as u32).unwrap();
        assert_eq!(profile.len(), 20 + 4 * (64 * 36 + 5 * 3));
        assert_profile_header(&profile, 5, 3, 5, 3);

        let thumbnail_offset = APOBS001_HEADER_BYTES;
        assert_eq!(
            profile_f32(&profile, thumbnail_offset + (12 * 64 + 32) * 4),
            (0.0722 * 255.0_f32) / 255.0
        );
        assert_eq!(
            profile_f32(&profile, thumbnail_offset + (24 * 64 + 51) * 4),
            (0.7152 * 255.0_f32) / 255.0
        );
        let grid_offset = thumbnail_offset + 64 * 36 * 4;
        assert_eq!(profile_f32(&profile, grid_offset + 7 * 4), 18.0_f32 / 255.0);
        assert_eq!(
            profile_f32(&profile, grid_offset + 14 * 4),
            182.0_f32 / 255.0
        );
        assert!(
            profile[grid_offset..]
                .chunks_exact(4)
                .all(|value| { f32::from_le_bytes(value.try_into().unwrap()).is_finite() })
        );
    }

    #[test]
    fn apobs001_large_plane_matches_source_derived_bounded_grid_golden() {
        let (width, height) = (2049_usize, 1025_usize);
        let mut rgba = vec![0; width * height * 4];
        for y in 0..height {
            set_pixel(&mut rgba, width, 12, y, [255, 255, 255, 255]);
            set_pixel(&mut rgba, width, 13, y, [255, 255, 255, 255]);
        }
        let profile = apobs001_profile(&rgba, width as u32, height as u32).unwrap();
        assert_eq!(profile.len(), APOBS001_MAX_BYTES);
        assert_profile_header(&profile, width as u32, height as u32, 128, 128);

        let thumbnail_offset = APOBS001_HEADER_BYTES;
        assert!(
            profile[thumbnail_offset..thumbnail_offset + 64 * 36 * 4]
                .chunks_exact(4)
                .all(|value| f32::from_le_bytes(value.try_into().unwrap()) == 0.0)
        );
        let grid_offset = thumbnail_offset + 64 * 36 * 4;
        for row in 0..128 {
            for column in 0..128 {
                let expected = if column == 0 { 0.2 } else { 0.0 };
                assert_eq!(
                    profile_f32(&profile, grid_offset + (row * 128 + column) * 4),
                    expected
                );
            }
        }
    }

    #[test]
    fn apobs001_rejects_invalid_plane_dimensions_and_lengths() {
        assert!(apobs001_profile(&[], 0, 1).is_none());
        assert!(apobs001_profile(&[], 1, 0).is_none());
        let wide_plane = vec![0; 4097 * 4];
        let wide_profile = apobs001_profile(&wide_plane, 4097, 1).unwrap();
        assert_profile_header(&wide_profile, 4097, 1, 128, 1);
        assert!(apobs001_profile(&[0; 3], 1, 1).is_none());
    }
}
