//! Fletcher 16-bit checksum implementation
pub struct Fletcher16 {
    a: u8,
    b: u8,
}

impl Fletcher16 {
    pub fn new() -> Self {
        Self { a: 0, b: 0 }
    }

    pub fn push_byte(&mut self, x: u8) {
        self.a = self.a.overflowing_add(x).0;
        self.b = self.b.overflowing_add(self.a).0;
    }

    pub fn push_slice(&mut self, data: &[u8]) {
        for x in data {
            self.push_byte(*x);
        }
    }

    pub fn value(&self) -> u16 {
        ((self.a as u16) << 8) | self.b as u16
    }

    pub fn compute(data: &[u8]) -> u16 {
        let mut chk = Self::new();
        chk.push_slice(data);
        chk.value()
    }
}
