/// BGRA patterns for mocks and demos.

/// Generates an animated BGRA gradient.
pub fn moving_gradient_bgra(width: u32, height: u32, frame: u64) -> Vec<u8> {
    let mut out = vec![0u8; width as usize * height as usize * 4];

    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;

            let fx = (x as u64 + frame) % 256;
            let fy = (y as u64 + frame / 2) % 256;

            // BGRA
            out[i + 0] = fx as u8;
            out[i + 1] = fy as u8;
            out[i + 2] = (255 - fx) as u8;
            out[i + 3] = 255;
        }
    }

    out
}
