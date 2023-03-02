use std::io::IoSlice;

use crate::htx::storage::HtxBuffer;

/// Intermediate representation for both H1 and H2 protocols
pub struct Htx<'a> {
    pub kind: HtxKind,
    pub storage: HtxBuffer<'a>,
    pub blocks: Vec<HtxBlock>,
    pub out: Vec<Store>,
    /// the start of the unparsed area in the buffer
    pub expects: usize,
    pub parsing_phase: HtxParsingPhase,
    pub body_size: HtxBodySize,
}

impl<'a> Htx<'a> {
    pub fn new(kind: HtxKind, storage: HtxBuffer<'a>) -> Self {
        Self {
            kind,
            blocks: Vec::new(),
            out: Vec::new(),
            expects: 0,
            parsing_phase: HtxParsingPhase::StatusLine,
            body_size: HtxBodySize::Empty,
            storage,
        }
    }

    pub fn push_left(&mut self, amount: u32) {
        for block in &mut self.blocks {
            block.push_left(amount);
        }
        for block in &mut self.out {
            block.push_left(amount);
        }
    }

    pub fn prepare(&mut self, converter: impl Fn(HtxBlock, &mut Vec<Store>)) {
        self.blocks
            .drain(..)
            .for_each(|block| converter(block, &mut self.out));
    }

    pub fn as_io_slice(&mut self) -> Vec<IoSlice> {
        self.out
            .iter()
            .map(|store| IoSlice::new(store.data(self.storage.buffer).expect("DATA")))
            .collect()
    }

    pub fn consume(&mut self, mut amount: usize) {
        let mut stores_left = Vec::new();
        let mut iter = self.out.drain(..);
        for store in iter.by_ref() {
            let (remaining, store) = store.consume(amount);
            amount = remaining;
            if let Some(store) = store {
                stores_left.push(store);
                break;
            }
        }
        assert!(amount == 0);

        stores_left.extend(iter);
        self.out = stores_left;

        let can_consume = self.leftmost_ref() - self.storage.start;
        self.storage.consume(can_consume);

        if self.storage.should_shift() {
            let amount = self.storage.shift() as u32;
            self.push_left(amount);
        }
    }

    pub fn leftmost_ref(&self) -> usize {
        for store in &self.out {
            if let Store::Slice(slice) = store {
                return slice.start as usize;
            }
        }
        self.storage.head
    }
}

#[derive(Debug, Clone, Copy)]
pub enum HtxKind {
    Request,
    Response,
}

#[derive(Debug, Clone, Copy)]
pub enum HtxParsingPhase {
    StatusLine,
    Headers,
    Body,
    Chunks,
    Trailers,
    Terminated,
    Error,
}

#[derive(Debug, Clone, Copy)]
pub enum HtxBodySize {
    Empty,
    Chunked,
    Length(usize),
}

#[derive(Debug)]
pub enum HtxBlock {
    StatusLine(StatusLine),
    Header(Header),
    Chunk(Chunk),
}

impl HtxBlock {
    pub fn push_left(&mut self, amount: u32) {
        match self {
            HtxBlock::StatusLine(StatusLine::Request {
                method,
                scheme,
                authority,
                path,
                uri,
                ..
            }) => {
                method.push_left(amount);
                scheme.push_left(amount);
                authority.push_left(amount);
                path.push_left(amount);
                uri.push_left(amount);
            }
            HtxBlock::StatusLine(StatusLine::Response { status, reason, .. }) => {
                status.push_left(amount);
                reason.push_left(amount);
            }
            HtxBlock::Header(header) => {
                header.key.push_left(amount);
                header.val.push_left(amount);
            }
            HtxBlock::Chunk(chunk) => {
                chunk.data.push_left(amount);
            }
        }
    }
}

#[derive(Debug)]
pub enum StatusLine {
    Request {
        version: Version,
        method: Store,
        scheme: Store,
        authority: Store,
        path: Store,
        uri: Store,
    },
    Response {
        version: Version,
        code: u16,
        status: Store,
        reason: Store,
    },
}

#[derive(Debug)]
pub struct Header {
    pub key: Store,
    pub val: Store,
}

#[derive(Debug)]
pub struct Chunk {
    pub data: Store,
}

#[derive(Debug)]
pub enum Store {
    Empty,
    Slice(Slice),
    Deported(Slice),
    Static(&'static [u8]),
    Vec(Vec<u8>, usize),
}

impl Store {
    pub fn new_slice(buffer: &[u8], data: &[u8]) -> Store {
        Store::Slice(Slice::new(buffer, data))
    }

    pub fn push_left(&mut self, amount: u32) {
        match self {
            Store::Slice(slice) => {
                slice.start -= amount;
            }
            Store::Deported(slice) => {
                slice.start -= amount;
            }
            _ => {}
        }
    }

    pub fn data<'a>(&'a self, buf: &'a [u8]) -> Option<&'a [u8]> {
        match self {
            Store::Empty => None,
            Store::Slice(slice) | Store::Deported(slice) => slice.data(buf),
            Store::Static(data) => Some(data),
            Store::Vec(data, index) => Some(&data[*index..]),
        }
    }

    pub fn modify(&mut self, buf: &mut [u8], new_value: &[u8]) {
        match &self {
            Store::Empty | Store::Deported(_) | Store::Static(_) | Store::Vec(..) => {
                println!("WARNING: modification is not expected on: {self:?}")
            }
            Store::Slice(_) => {}
        }
        match self {
            Store::Empty | Store::Static(_) => *self = Store::Vec(new_value.to_vec(), 0),
            Store::Slice(slice) | Store::Deported(slice) => {
                let new_len = new_value.len();
                if slice.len() >= new_len {
                    let start = slice.start as usize;
                    let end = start + new_len;
                    buf[start..end].copy_from_slice(new_value);
                    slice.len = new_len as u32;
                } else {
                    *self = Store::Vec(new_value.to_vec(), 0)
                }
            }
            Store::Vec(vec, _) => {
                vec.clear();
                vec.extend_from_slice(new_value);
            }
        }
    }

    pub fn consume(self, amount: usize) -> (usize, Option<Store>) {
        match self {
            Store::Empty => (amount, None),
            Store::Slice(slice) => {
                let (remaining, opt) = slice.consume(amount);
                (remaining, opt.map(Store::Slice))
            }
            Store::Deported(slice) => {
                let (remaining, opt) = slice.consume(amount);
                (remaining, opt.map(Store::Slice))
            }
            Store::Static(data) => {
                if amount >= data.len() {
                    (amount - data.len(), None)
                } else {
                    (0, Some(Store::Static(&data[amount..])))
                }
            }
            Store::Vec(data, index) => {
                if amount >= data.len() - index {
                    (amount - data.len() + index, None)
                } else {
                    (0, Some(Store::Vec(data, index + amount)))
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Slice {
    pub start: u32,
    pub len: u32,
}

impl Slice {
    // data MUST be a subset of buffer
    pub fn new(buffer: &[u8], data: &[u8]) -> Slice {
        let offset = data.as_ptr() as usize - buffer.as_ptr() as usize;
        assert!(
            offset <= u32::MAX as usize,
            "slices should not start at more than 4GB from its beginning"
        );
        assert!(
            data.len() <= u16::MAX as usize,
            "slices should not be larger than 65536 bytes"
        );

        Slice {
            start: offset as u32,
            len: data.len() as u32,
        }
    }

    pub fn data<'a>(&self, buffer: &'a [u8]) -> Option<&'a [u8]> {
        let start = self.start as usize;
        let end = start + self.len();

        if start <= buffer.len() && end <= buffer.len() {
            Some(&buffer[start..end])
        } else {
            None
        }
    }

    pub fn consume(self, amount: usize) -> (usize, Option<Slice>) {
        if amount >= self.len() {
            (amount - self.len(), None)
        } else {
            let Slice { start, len } = self;
            (
                0,
                Some(Slice {
                    start: start + (amount as u32),
                    len: len - (amount as u32),
                }),
            )
        }
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum Version {
    V10,
    V11,
    V20,
}