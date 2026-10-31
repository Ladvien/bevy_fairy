//! A contact sheet: several captured frames in one PNG, each cell labelled with what the
//! plugin reported for it (its game frame) and when it was delivered.
//!
//! The labels are drawn from a 3x5 bitmap font so the server needs no font file. Only the
//! glyphs the labels use exist; any other character is refused rather than drawn as a blank.

use image::{imageops, Rgba, RgbaImage};

/// Cells per row. A sheet of at most 8 cells is therefore at most 2 rows by 4 columns.
pub const COLUMNS: u32 = 4;
/// Most cells one sheet holds.
pub const MAX_CELLS: usize = 8;
/// Width of one cell's picture in pixels; frames are scaled down to it, never up.
pub const CELL_WIDTH: u32 = 320;

const SCALE: u32 = 2;
const GLYPH_W: u32 = 3;
const GLYPH_H: u32 = 5;
const PAD: u32 = 3;
const LABEL_H: u32 = GLYPH_H * SCALE + PAD * 2;
const BACKGROUND: Rgba<u8> = Rgba([24, 24, 24, 255]);
const INK: Rgba<u8> = Rgba([255, 255, 255, 255]);

/// One cell: the frame and its label (`0-9 F M S R E + -` and space).
pub struct Cell {
    pub image: RgbaImage,
    pub label: String,
}

/// Rows of three bits, top to bottom, for the one glyph; `None` for a character not in the font.
fn glyph(c: char) -> Option<[u8; 5]> {
    Some(match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' | 'S' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        ' ' => [0; 5],
        _ => return None,
    })
}

fn draw_label(sheet: &mut RgbaImage, x0: u32, y0: u32, label: &str) {
    let mut x = x0 + PAD;
    for c in label.chars() {
        let rows = glyph(c).expect("labels are checked before drawing");
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..GLYPH_W {
                if bits >> (GLYPH_W - 1 - col) & 1 == 1 {
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let px = x + col * SCALE + dx;
                            let py = y0 + PAD + row as u32 * SCALE + dy;
                            if px < sheet.width() && py < sheet.height() {
                                sheet.put_pixel(px, py, INK);
                            }
                        }
                    }
                }
            }
        }
        x += (GLYPH_W + 1) * SCALE;
    }
}

/// Pixels a label needs: its glyphs, `PAD` on both sides.
fn label_width(label: &str) -> u32 {
    let glyphs = label.chars().count() as u32;
    if glyphs == 0 {
        return 0;
    }
    PAD + (glyphs - 1) * (GLYPH_W + 1) * SCALE + GLYPH_W * SCALE + PAD
}

/// Height of one cell's picture once scaled to `CELL_WIDTH` (or left as it is when narrower).
fn scaled_size(width: u32, height: u32) -> (u32, u32) {
    if width <= CELL_WIDTH {
        return (width, height);
    }
    (CELL_WIDTH, ((u64::from(height) * u64::from(CELL_WIDTH)).div_ceil(u64::from(width))).max(1) as u32)
}

/// Lay `cells` out left to right, top to bottom, `COLUMNS` to a row, each under its label.
///
/// A cell is as wide as its picture, or as its widest label when that is wider (a 100 px capture is
/// narrower than `F123456 +1234MS`): a label is never clipped at the sheet's edge or drawn into the
/// next cell, because the sheet then reads a different time than the one the reply states. The
/// picture sits at the cell's left and the rest of a wider cell is background.
///
/// Refuses an empty list, more than `MAX_CELLS`, cells of unequal size (a sheet of differently
/// shaped frames would misalign), and a label with a character outside the font.
pub fn compose(cells: &[Cell]) -> Result<RgbaImage, String> {
    if cells.is_empty() || cells.len() > MAX_CELLS {
        return Err(format!("a contact sheet holds 1 to {MAX_CELLS} cells, got {}", cells.len()));
    }
    let (w, h) = cells[0].image.dimensions();
    if let Some(bad) = cells.iter().find(|c| c.image.dimensions() != (w, h)) {
        let (bw, bh) = bad.image.dimensions();
        return Err(format!("frames differ in size ({w}x{h} and {bw}x{bh}); the game was resized mid-burst"));
    }
    if let Some(bad) = cells.iter().find_map(|c| c.label.chars().find(|ch| glyph(*ch).is_none())) {
        return Err(format!("label character {bad:?} is not in the sheet font"));
    }
    let (cw, ch) = scaled_size(w, h);
    let columns = (cells.len() as u32).min(COLUMNS);
    let rows = (cells.len() as u32).div_ceil(COLUMNS);
    let widest_label = cells.iter().map(|c| label_width(&c.label)).max().unwrap_or(0);
    let (stride_x, stride_y) = (cw.max(widest_label), LABEL_H + ch);
    let mut sheet = RgbaImage::from_pixel(columns * stride_x, rows * stride_y, BACKGROUND);
    for (i, cell) in cells.iter().enumerate() {
        let (x0, y0) = ((i as u32 % COLUMNS) * stride_x, (i as u32 / COLUMNS) * stride_y);
        draw_label(&mut sheet, x0, y0, &cell.label);
        if (cw, ch) == (w, h) {
            imageops::replace(&mut sheet, &cell.image, i64::from(x0), i64::from(y0 + LABEL_H));
        } else {
            let small = imageops::resize(&cell.image, cw, ch, imageops::FilterType::Triangle);
            imageops::replace(&mut sheet, &small, i64::from(x0), i64::from(y0 + LABEL_H));
        }
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255]))
    }

    fn cell(rgb: [u8; 3], label: &str) -> Cell {
        Cell { image: flat(40, 30, rgb), label: label.to_string() }
    }

    #[test]
    fn eight_cells_make_two_rows_of_four_with_each_frame_in_its_own_slot() {
        let colours = [[10, 0, 0], [20, 0, 0], [30, 0, 0], [40, 0, 0], [50, 0, 0], [60, 0, 0], [70, 0, 0], [80, 0, 0]];
        let cells: Vec<Cell> = colours.iter().map(|c| cell(*c, "F1")).collect();
        let sheet = compose(&cells).expect("8 equal cells compose");
        assert_eq!(sheet.dimensions(), (4 * 40, 2 * (LABEL_H + 30)));
        for (i, c) in colours.iter().enumerate() {
            let (x, y) = ((i as u32 % 4) * 40 + 20, (i as u32 / 4) * (LABEL_H + 30) + LABEL_H + 15);
            assert_eq!(sheet.get_pixel(x, y).0[..3], c[..], "cell {i} holds its own frame");
        }
    }

    #[test]
    fn a_label_draws_ink_above_its_frame_and_different_labels_differ() {
        let one = compose(&[cell([0, 0, 90], "F1")]).expect("composes");
        let two = compose(&[cell([0, 0, 90], "F2")]).expect("composes");
        let ink = |s: &RgbaImage| (0..LABEL_H).flat_map(|y| (0..40).map(move |x| (x, y))).filter(|&(x, y)| *s.get_pixel(x, y) == INK).count();
        assert!(ink(&one) > 0, "the label strip has white pixels");
        assert_ne!(one.as_raw(), two.as_raw(), "F1 and F2 are drawn differently");
    }

    /// B-132: a cell is as wide as its frame, and a 100 px capture is narrower than
    /// `F123456 +1234MS`, so the label was clipped at the sheet's right edge in the last cell and
    /// spilled into the next cell's strip in every other one -- the sheet then read `+12` for
    /// `+1234MS`. The reference is the label drawn on a canvas with room to spare.
    #[test]
    fn a_label_wider_than_its_frame_is_drawn_whole_and_stays_in_its_own_cell() {
        let label = "F123456 +1234MS";
        let cells = [cell([0, 0, 90], label), cell([0, 0, 90], label), cell([0, 0, 90], label)];
        let sheet = compose(&cells).expect("composes");
        let mut want = RgbaImage::from_pixel(400, LABEL_H, BACKGROUND);
        draw_label(&mut want, 0, 0, label);
        let needed = (0..want.width()).rev().find(|&x| (0..LABEL_H).any(|y| *want.get_pixel(x, y) == INK)).expect("the label draws") + 1;
        let stride = sheet.width() / cells.len() as u32;
        assert!(stride >= needed, "a cell is {stride} px wide and its label needs {needed}");
        for i in 0..cells.len() as u32 {
            for y in 0..LABEL_H {
                for x in 0..needed {
                    assert_eq!(sheet.get_pixel(i * stride + x, y), want.get_pixel(x, y), "cell {i}, label pixel ({x},{y})");
                }
            }
        }
    }

    #[test]
    fn wide_frames_are_scaled_to_the_cell_width() {
        let wide = Cell { image: flat(1280, 720, [5, 5, 5]), label: "REF F9".into() };
        let sheet = compose(&[wide]).expect("composes");
        assert_eq!(sheet.dimensions(), (320, LABEL_H + 180));
    }

    #[test]
    fn refuses_what_it_cannot_lay_out_honestly() {
        assert!(compose(&[]).is_err());
        let nine: Vec<Cell> = (0..9).map(|_| cell([0; 3], "F1")).collect();
        assert!(compose(&nine).is_err());
        let mixed = [cell([0; 3], "F1"), Cell { image: flat(41, 30, [0; 3]), label: "F2".into() }];
        assert!(compose(&mixed).unwrap_err().contains("differ in size"));
        assert!(compose(&[cell([0; 3], "frame")]).unwrap_err().contains("not in the sheet font"));
    }
}
