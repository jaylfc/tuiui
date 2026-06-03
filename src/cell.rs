#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba { pub r: u8, pub g: u8, pub b: u8, pub a: u8 }

impl Rgba {
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self { Self { r, g, b, a } }
    pub fn rgb(r: u8, g: u8, b: u8) -> Self { Self { r, g, b, a: 255 } }
    pub const TRANSPARENT: Rgba = Rgba { r: 0, g: 0, b: 0, a: 0 };

    /// Porter-Duff "over": self composited on top of `dst`. Result is opaque.
    pub fn over(self, dst: Rgba) -> Rgba {
        if self.a == 255 { return Rgba::rgb(self.r, self.g, self.b); }
        if self.a == 0 { return Rgba::rgb(dst.r, dst.g, dst.b); }
        let a = self.a as u32;
        let inv = 255 - a;
        let mix = |s: u8, d: u8| -> u8 { ((s as u32 * a + d as u32 * inv) / 255) as u8 };
        Rgba::rgb(mix(self.r, dst.r), mix(self.g, dst.g), mix(self.b, dst.b))
    }
    /// Multiply this color's alpha by `opacity` (0.0–1.0).
    pub fn with_opacity(self, opacity: f32) -> Rgba {
        let a = (self.a as f32 * opacity).round().clamp(0.0, 255.0) as u8;
        Rgba::new(self.r, self.g, self.b, a)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CellAttrs { pub bold: bool, pub italic: bool, pub underline: bool, pub inverse: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell { pub ch: char, pub fg: Rgba, pub bg: Rgba, pub attrs: CellAttrs }

impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', fg: Rgba::rgb(200, 208, 220), bg: Rgba::TRANSPARENT, attrs: CellAttrs::default() }
    }
}
