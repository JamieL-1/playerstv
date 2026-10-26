//! The test game's picture. Every frame is a pure function of its index, so
//! the analyzer can read which frame it's looking at and redraw the exact
//! reference to compare quality against.
//!
//! Layout (sizes scale with the frame, so a recorder that rescales still
//! decodes):
//! - a strip along the top with the frame index as 32 black or white blocks:
//!   24 bits of index, then an 8-bit check;
//! - a square under it that's white on the first frame of every second, for
//!   lining video up with the audio click;
//! - a scrolling gradient everywhere else, with a moving sprite, a block of
//!   per-frame noise and a scrolling "UI text" area on top. These give the
//!   encoder motion, fine detail and flat areas to work on, like a game.

pub const BLOCKS: usize = 32;
const INDEX_BITS: u32 = 24;
const INDEX_MASK: u64 = (1 << INDEX_BITS) - 1;
const TEXT_BG: u8 = 220;
const TEXT_FG: u8 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pattern {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Pattern {
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        assert!(width >= BLOCKS as u32 * 4 && height >= 64 && fps > 0, "pattern too small: {width}x{height}@{fps}");
        Self { width, height, fps }
    }

    /// Height of the index strip. The analyzer crops `ih/48`, which stays
    /// inside it for any scale.
    pub fn strip_height(&self) -> u32 {
        (self.height / 40).max(8)
    }

    pub fn is_flash(&self, index: u64) -> bool {
        index.is_multiple_of(self.fps as u64)
    }

    pub fn frame_bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Draws frame `index` as BGRA into `buf` (`frame_bytes()` long).
    pub fn render(&self, index: u64, buf: &mut [u8]) {
        assert_eq!(buf.len(), self.frame_bytes());
        let (w, h) = (self.width as usize, self.height as usize);
        let i = index as usize;
        let strip = self.strip_height() as usize;
        let flash = h / 8;
        let bits = encode(index);
        let block_w = w / BLOCKS;

        let sprite = h / 6;
        let phase = (index % 240) as f64 / 240.0 * std::f64::consts::TAU;
        let sprite_x = ((w - sprite) as f64 * (0.5 + 0.45 * phase.sin())) as usize;
        let sprite_y = strip + ((h - strip - sprite) as f64 * (0.5 + 0.4 * (phase * 2.0).cos())) as usize;

        let noise = (w / 8, h / 2, w / 6, h / 6); // x, y, width, height
        let text = (w / 2, h / 2); // the bottom-right quarter

        for (y, row) in buf.chunks_exact_mut(w * 4).enumerate() {
            let px = row.as_chunks_mut::<4>().0;
            if y < strip {
                for (x, p) in px.iter_mut().enumerate() {
                    let block = (x / block_w).min(BLOCKS - 1);
                    let on = bits >> (BLOCKS - 1 - block) & 1 == 1;
                    *p = gray(if on { 255 } else { 0 });
                }
                continue;
            }

            for (x, p) in px.iter_mut().enumerate() {
                *p = [(x + 4 * i) as u8, (y + 2 * i) as u8, ((x + y) / 2 + i) as u8, 255];
            }
            if (sprite_y..sprite_y + sprite).contains(&y) {
                let shade = ((y - sprite_y) * 255 / sprite) as u8;
                px[sprite_x..sprite_x + sprite].fill([40, shade, 255 - shade, 255]);
            }
            if (noise.1..noise.1 + noise.3).contains(&y) {
                let mut state = splitmix(index << 32 ^ y as u64);
                for p in &mut px[noise.0..noise.0 + noise.2] {
                    state = xorshift(state);
                    let v = state.to_le_bytes();
                    *p = [v[0], v[1], v[2], 255];
                }
            }
            if y >= text.1 {
                // 8×12 glyph cells scrolling up one pixel per frame.
                let ty = y + i;
                let (cell_row, glyph_row) = (ty / 12, ty % 12);
                for (cx, cell) in px[text.0..].chunks_mut(8).enumerate() {
                    let ink = if glyph_row >= 10 {
                        0 // line spacing
                    } else {
                        splitmix((cell_row as u64) << 32 ^ (cx as u64) << 8 ^ glyph_row as u64) as u8 & 0b0111_1110
                    };
                    for (bit, p) in cell.iter_mut().enumerate() {
                        *p = gray(if ink >> bit & 1 == 1 { TEXT_FG } else { TEXT_BG });
                    }
                }
            }
            // Drawn last so the sprite never covers it.
            if y < strip + flash {
                px[..flash].fill(gray(if self.is_flash(index) { 255 } else { 0 }));
            }
        }
    }
}

fn gray(v: u8) -> [u8; 4] {
    [v, v, v, 255]
}

fn check(index: u64) -> u32 {
    let b = (index & INDEX_MASK).to_le_bytes();
    (b[0] ^ b[1] ^ b[2] ^ 0xA5) as u32
}

/// The strip's 32 bits for `index`, most significant block first.
pub fn encode(index: u64) -> u32 {
    ((index & INDEX_MASK) as u32) << 8 | check(index)
}

/// Reads the frame index from the average brightness of each of the
/// [`BLOCKS`] strip blocks. `None` if the check doesn't match (not our
/// picture, or too damaged to trust).
pub fn decode(blocks: &[u8; BLOCKS]) -> Option<u64> {
    let bits = blocks.iter().fold(0u32, |acc, &b| acc << 1 | (b >= 128) as u32);
    let index = (bits >> 8) as u64;
    (bits & 0xFF == check(index)).then_some(index)
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn xorshift(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^ x << 17
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the analyzer gets: the strip scaled to one pixel per block.
    fn blocks_of(p: &Pattern, frame: &[u8]) -> [u8; BLOCKS] {
        let w = p.width as usize;
        let block_w = w / BLOCKS;
        let row = p.strip_height() as usize / 2;
        std::array::from_fn(|b| {
            let px = &frame[(row * w + b * block_w) * 4..(row * w + (b + 1) * block_w) * 4];
            (px.chunks(4).map(|c| c[1] as usize).sum::<usize>() / block_w) as u8
        })
    }

    #[test]
    fn index_round_trips_through_the_picture() {
        let p = Pattern::new(1920, 1080, 60);
        let mut frame = vec![0; p.frame_bytes()];
        for index in [0, 1, 59, 60, 12_345, INDEX_MASK] {
            p.render(index, &mut frame);
            assert_eq!(decode(&blocks_of(&p, &frame)), Some(index));
        }
    }

    #[test]
    fn decode_survives_encoder_noise_but_rejects_garbage() {
        let mut blocks: [u8; BLOCKS] =
            std::array::from_fn(|b| if encode(4242) >> (BLOCKS - 1 - b) & 1 == 1 { 255 } else { 0 });
        // Lossy coding pulls blacks up and whites down a long way.
        for (b, v) in blocks.iter_mut().enumerate() {
            *v = if *v == 255 { 255 - (b as u8 * 3) } else { 16 + b as u8 * 2 };
        }
        assert_eq!(decode(&blocks), Some(4242));
        // A flipped bit fails the check instead of reading as another frame.
        blocks[5] = 255 - blocks[5];
        assert_eq!(decode(&blocks), None);
        assert_eq!(decode(&[128; BLOCKS]), None);
    }

    #[test]
    fn frames_are_deterministic_and_differ() {
        let p = Pattern::new(640, 360, 60);
        let (mut a, mut b) = (vec![0; p.frame_bytes()], vec![0; p.frame_bytes()]);
        p.render(7, &mut a);
        p.render(7, &mut b);
        assert_eq!(a, b);
        p.render(8, &mut b);
        assert_ne!(a, b);
    }

    #[test]
    fn flash_once_a_second() {
        let p = Pattern::new(640, 360, 60);
        let flashes: Vec<u64> = (0..180).filter(|&i| p.is_flash(i)).collect();
        assert_eq!(flashes, [0, 60, 120]);
    }
}
