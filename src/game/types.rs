use ndarray::Array2;

/// Linear dimension, used for individual coordinates or minefield width/height
pub type Ix = u8;

/// Area dimension, used for mine/tile counts
pub type Ax = u16;

/// Shorthand for position/size with Ix
pub type Ix2 = (Ix, Ix);

pub trait NdConvert {
    type Output;
    fn convert(self) -> Self::Output;
}

impl NdConvert for Ix2 {
    type Output = [usize; 2];
    fn convert(self) -> Self::Output {
        [self.0.into(), self.1.into()]
    }
}

pub const fn mult(a: Ix, b: Ix) -> Ax {
    let a = a as Ax;
    let b = b as Ax;
    a.saturating_mul(b)
}

pub trait AdjacentIterator {
    // XXX: returning a impl Iterator seems to imply a &self borrow, using concrete type for now
    //fn iter_adjacent(&self, index: Ix2) -> impl Iterator<Item = Ix2>;
    fn iter_adjacent(&self, index: Ix2) -> IterAdjacent;
}

impl<T> AdjacentIterator for Array2<T> {
    //fn iter_adjacent(&self, index: Ix2) -> impl Iterator<Item = Ix2> {
    fn iter_adjacent(&self, index: Ix2) -> IterAdjacent {
        let dim = self.dim();
        let size = (dim.0.try_into().unwrap(), dim.1.try_into().unwrap());
        IterAdjacent::new(index, size)
    }
}

pub trait AdjacentTileIterator<T>: AdjacentIterator {
    fn iter_adjacent_tiles_with_index(&self, index: Ix2) -> impl Iterator<Item = (Ix2, T)>;
    fn iter_adjacent_tiles(&self, index: Ix2) -> impl Iterator<Item = T> {
        self.iter_adjacent_tiles_with_index(index).map(|(_, tile)| tile)
    }
}

impl<T: Copy> AdjacentTileIterator<T> for Array2<T> {
    fn iter_adjacent_tiles_with_index(&self, index: Ix2) -> impl Iterator<Item = (Ix2, T)> {
        self.iter_adjacent(index).map(|index| (index, self[index.convert()]))
    }
}

// Define a displacement mapping for each direction
const DISPLACEMENTS: [(isize, isize); 8] = [
    (-1, -1), // Top-Left
    (0, -1),  // Top
    (1, -1),  // Top-Right
    (-1, 0),  // Left
    (1, 0),   // Right
    (-1, 1),  // Bottom-Left
    (0, 1),   // Bottom
    (1, 1),   // Bottom-Right
];

/// Will make coords + delta and return the result if it is withing bounds
fn apply_delta(coords: Ix2, delta: (isize, isize), bounds: Ix2) -> Option<Ix2> {
    let (x, y) = coords;
    let (dx, dy) = delta;
    let (bx, by) = bounds;
    let nx = x.checked_add_signed(dx.try_into().ok()?)?;
    if nx >= bx {
        return None;
    }
    let ny = y.checked_add_signed(dy.try_into().ok()?)?;
    if ny >= by {
        return None;
    }
    Some((nx, ny))
}

#[derive(Debug)]
pub struct IterAdjacent {
    center: Ix2,
    bounds: Ix2,
    index: u8,
}

impl IterAdjacent {
    fn new(center: Ix2, bounds: Ix2) -> Self {
        IterAdjacent {
            center,
            bounds,
            index: 0,
        }
    }
}

impl Iterator for IterAdjacent {
    type Item = Ix2;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if usize::from(self.index) >= DISPLACEMENTS.len() {
                return None;
            }
            let next_item = apply_delta(self.center, DISPLACEMENTS[self.index as usize], self.bounds);
            self.index += 1;
            if next_item.is_some() {
                return next_item;
            }
        }
    }
}
