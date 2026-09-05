pub mod activity;
pub mod identity;
pub mod jsonl_reader;
mod normalize;
pub use normalize::Parser;

use crate::domain::Session;
use jsonl_reader::BoundedReader;
use std::{io, path::Path};

pub struct Tail {
    pub reader: BoundedReader,
    pub parser: Parser,
}
impl Tail {
    pub fn open(path: &Path, max_record_bytes: usize, preview_width: usize) -> io::Result<Self> {
        Ok(Self {
            reader: BoundedReader::open(path, max_record_bytes)?,
            parser: Parser::new(preview_width),
        })
    }
    pub fn poll(&mut self, budget: usize) -> io::Result<(usize, bool)> {
        let reset = self.reader.check_reset()?;
        if reset {
            self.parser = Parser::new(self.parser.preview_width);
        }
        let bytes = self
            .reader
            .read_batch(budget, |record| self.parser.consume(record))?;
        Ok((bytes, reset))
    }
    pub fn session(&self) -> &Session {
        &self.parser.session
    }
}
