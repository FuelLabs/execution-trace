use std::{cell::RefCell, io::Read};

use fuel_vm::{fuel_asm::Word, prelude::MemoryInstance};

#[derive(Clone)]
pub struct MemoryReader<'a> {
    mem: &'a MemoryInstance,
    at: RefCell<Word>,
}

impl<'a> MemoryReader<'a> {
    pub fn new(mem: &'a MemoryInstance, at: Word) -> Self {
        Self {
            mem,
            at: RefCell::new(at),
        }
    }
}

impl Read for MemoryReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let at = self.at.replace_with(|at| *at + buf.len() as Word);
        buf.copy_from_slice(self.mem.read(at, buf.len()).map_err(|err| {
            std::io::Error::new(std::io::ErrorKind::Other, "Inaccessible memory")
        })?);
        Ok(buf.len())
    }
}
