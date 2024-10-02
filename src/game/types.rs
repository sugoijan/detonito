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
