use ndarray::Array2;

/// Single coordinate axis used for board width, height, and positions.
pub type Coord = u8;

/// Count type used for mine counts and total-cell counts.
pub type CellCount = u16;

/// Two-dimensional coordinates `(x, y)`.
pub type Coord2 = (Coord, Coord);

pub trait ToNdIndex {
    type Output;
    fn to_nd_index(self) -> Self::Output;
}

impl ToNdIndex for Coord2 {
    type Output = [usize; 2];

    fn to_nd_index(self) -> Self::Output {
        [self.0.into(), self.1.into()]
    }
}

pub const fn mult(a: Coord, b: Coord) -> CellCount {
    let a = a as CellCount;
    let b = b as CellCount;
    a.saturating_mul(b)
}

pub trait NeighborIterExt {
    fn iter_neighbors(&self, index: Coord2) -> NeighborIter;
}

impl<T> NeighborIterExt for Array2<T> {
    fn iter_neighbors(&self, index: Coord2) -> NeighborIter {
        let dim = self.dim();
        let size = (dim.0.try_into().unwrap(), dim.1.try_into().unwrap());
        NeighborIter::new(index, size)
    }
}

pub trait NeighborCellIterExt<T>: NeighborIterExt {
    fn iter_neighbor_cells_with_index(&self, index: Coord2) -> impl Iterator<Item = (Coord2, T)>;

    fn iter_neighbor_cells(&self, index: Coord2) -> impl Iterator<Item = T> {
        self.iter_neighbor_cells_with_index(index)
            .map(|(_, cell)| cell)
    }
}

impl<T: Copy> NeighborCellIterExt<T> for Array2<T> {
    fn iter_neighbor_cells_with_index(&self, index: Coord2) -> impl Iterator<Item = (Coord2, T)> {
        self.iter_neighbors(index)
            .map(|index| (index, self[index.to_nd_index()]))
    }
}

const DISPLACEMENTS: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Applies `delta` to `coords`, returning a value only when it remains in bounds.
fn apply_delta(coords: Coord2, delta: (isize, isize), bounds: Coord2) -> Option<Coord2> {
    let (x, y) = coords;
    let (dx, dy) = delta;
    let (max_x, max_y) = bounds;

    let next_x = x.checked_add_signed(dx.try_into().ok()?)?;
    if next_x >= max_x {
        return None;
    }

    let next_y = y.checked_add_signed(dy.try_into().ok()?)?;
    if next_y >= max_y {
        return None;
    }

    Some((next_x, next_y))
}

#[derive(Debug)]
pub struct NeighborIter {
    center: Coord2,
    bounds: Coord2,
    index: u8,
}

impl NeighborIter {
    fn new(center: Coord2, bounds: Coord2) -> Self {
        Self {
            center,
            bounds,
            index: 0,
        }
    }
}

impl Iterator for NeighborIter {
    type Item = Coord2;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if usize::from(self.index) >= DISPLACEMENTS.len() {
                return None;
            }

            let next_item =
                apply_delta(self.center, DISPLACEMENTS[self.index as usize], self.bounds);
            self.index += 1;

            if next_item.is_some() {
                return next_item;
            }
        }
    }
}
