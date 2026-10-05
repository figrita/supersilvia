// SPDX-License-Identifier: AGPL-3.0-or-later

//! Straight colors made premultiplied, at the boundary they enter the graph by.
//!
//! Every color inside the graph is premultiplied, and a color picker, a CPU node's published
//! color, a PNG, a GIF and the drawing canvas's own buffer are straight
//! ([decisions.md](../../../docs/decisions.md#colors-in-the-graph-are-premultiplied)). These are
//! the one conversion each of them goes through: where the synth resolves a color into a
//! uniform, and where a straight picture is handed to the renderer. A data texture whose alpha
//! is not coverage — a simulation's state, an audio meter — goes through neither.
//!
//! An opaque color is its own premultiplied form, bit for bit, so a patch with nothing
//! transparent in it draws what it would have drawn straight.

/// A straight color premultiplied: its color channels times its alpha.
pub fn premultiply([r, g, b, a]: [f32; 4]) -> [f32; 4] {
    [r * a, g * a, b * a, a]
}

/// Straight RGBA8 texels premultiplied in place, each channel rounded to the nearest byte.
/// A length that is not a whole number of texels leaves the remainder as it is.
pub fn premultiply_rgba8(rgba: &mut [u8]) {
    for texel in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(texel[3]);
        if a == 255 {
            continue;
        }
        // c · a / 255 is never a half, 255 being odd, so adding 127 rounds it.
        for c in &mut texel[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opaque_color_is_its_own_premultiplied_form() {
        let c = [0.3, 0.7, 0.123_456_7, 1.0];
        assert_eq!(premultiply(c), c);
        let mut bytes = [10, 200, 30, 255];
        premultiply_rgba8(&mut bytes);
        assert_eq!(bytes, [10, 200, 30, 255]);
    }

    #[test]
    fn a_byte_is_rounded_to_the_nearest() {
        let mut bytes = [200, 100, 51, 128, 255, 255, 255, 0, 255, 1, 128, 1];
        premultiply_rgba8(&mut bytes);
        assert_eq!(bytes, [100, 50, 26, 128, 0, 0, 0, 0, 1, 0, 1, 1]);
        for c in 0..=255u32 {
            for a in 0..=255u32 {
                let mut texel = [c as u8, 0, 0, a as u8];
                premultiply_rgba8(&mut texel);
                let exact = f64::from(c) * f64::from(a) / 255.0;
                assert_eq!(f64::from(texel[0]), exact.round(), "{c} at {a}");
            }
        }
    }
}
