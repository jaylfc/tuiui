use crate::cell::{Cell, Rgba};

#[derive(Clone, Debug)]
pub struct CellBuffer { w: i32, h: i32, cells: Vec<Cell> }

impl CellBuffer {
    pub fn new(w: i32, h: i32) -> Self {
        let (w, h) = (w.max(0), h.max(0));
        Self { w, h, cells: vec![Cell::default(); (w * h) as usize] }
    }
    pub fn width(&self) -> i32 { self.w }
    pub fn height(&self) -> i32 { self.h }
    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h { None }
        else { Some((y * self.w + x) as usize) }
    }
    pub fn get(&self, x: i32, y: i32) -> Option<&Cell> { self.idx(x, y).map(|i| &self.cells[i]) }
    pub fn set(&mut self, x: i32, y: i32, c: Cell) { if let Some(i) = self.idx(x, y) { self.cells[i] = c; } }
    pub fn fill(&mut self, c: Cell) { for cell in &mut self.cells { *cell = c; } }
    pub fn write_str(&mut self, x: i32, y: i32, s: &str, fg: Rgba, bg: Rgba) {
        for (i, ch) in s.chars().enumerate() {
            self.set(x + i as i32, y, Cell { ch, fg, bg, attrs: Default::default() });
        }
    }
}
