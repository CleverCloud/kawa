use std::{collections::VecDeque, io::IoSlice};

use crate::storage::{AsBuffer, HtxBlockConverter, HtxBuffer};

/// Intermediate representation for both H1 and H2 protocols
///
/// /!\ note: the blocks and out fields should always contains "exclusive" data. More specifically
/// out should always contain "older" data than blocks. This is an invariant of the prepare method.
pub struct Htx<T: AsBuffer> {
    pub storage: HtxBuffer<T>,
    /// Protocol independant representation of the parsed data in the HtxBuffer
    pub blocks: VecDeque<HtxBlock>,
    /// Protocol dependant representation generated from the Htx representation in blocks
    pub out: VecDeque<OutBlock>,

    // Those 4 last fields are set and used by external parsers,
    // Htx doesn't use them directly.
    pub kind: Kind,
    pub expects: usize,
    pub parsing_phase: ParsingPhase,
    pub body_size: BodySize,
}

impl<T: AsBuffer> Htx<T> {
    /// Create a new Htx struct around a given storage.
    ///
    /// note: the storage is moved into Htx and shouldn't be directly accessed after that point.
    /// You can retrieve it right before dropping Htx.
    pub fn new(kind: Kind, storage: HtxBuffer<T>) -> Self {
        Self {
            kind,
            blocks: VecDeque::new(),
            out: VecDeque::new(),
            expects: 0,
            parsing_phase: ParsingPhase::StatusLine,
            body_size: BodySize::Empty,
            storage,
        }
    }

    /// Synchronize back all the Stores from blocks and out with the underlying data of HtxBuffer.
    /// This is necessary after a HtxBuffer::shift.
    pub fn push_left(&mut self, amount: u32) {
        for block in &mut self.blocks {
            block.push_left(amount);
        }
        for block in &mut self.out {
            block.push_left(amount);
        }
    }

    /// Convert Htx representation from blocks to a protocol specific representation in out.
    /// HtxBlockConverter takes block one by one and should push Stores in the out vector using
    /// dedicated push_out method or push_back on the out field. HtxBlockConverter allows to
    /// implement stateful behavior.
    ///
    /// /!\ note: the interface can seem restrictive, but it enforces some invariants, some that
    /// might not be appearant at first.
    ///
    /// note 2: converters can push delimiters in the out vector (via push_delimiter) to fragment
    /// the "stream". This can be used to split H2 frames.
    pub fn prepare<C: HtxBlockConverter<T>>(&mut self, converter: &mut C) {
        converter.initialize(self);
        while let Some(block) = self.blocks.pop_front() {
            converter.call(block, self);
        }
        converter.finalize(self);
    }

    /// Return a vector of IoSlices collecting every bytes from the out vector up to its end or a
    /// delimiter: OutBlock::Delimiter. This can be used to split H2 frames.
    ///
    /// note: until you drop the resulting vector, Rust will prevent mutably borrowing Htx as the
    /// IoSlices keep a reference in the out vector. As always, nothing is copied.
    pub fn as_io_slice(&mut self) -> Vec<IoSlice> {
        self.out
            .iter()
            .take_while(|block| match block {
                OutBlock::Delimiter => false,
                OutBlock::Store(_) => true,
            })
            .map(|block| match block {
                OutBlock::Delimiter => unreachable!(), // due to previous take_while
                OutBlock::Store(store) => IoSlice::new(store.data(self.storage.buffer())),
            })
            .collect()
    }

    /// Given an amount of bytes consumed, this method removes the relevant OutBlocks from the out
    /// vector and truncates any partially consumed block. It manages the underlying HtxBuffer,
    /// shifting and synchronizing the data if it deems appropriate.
    pub fn consume(&mut self, mut amount: usize) {
        while let Some(store) = self.out.pop_front() {
            let (remaining, store) = store.consume(amount);
            amount = remaining;
            if let Some(store) = store {
                self.out.push_front(OutBlock::Store(store));
                break;
            }
        }
        assert!(amount == 0);

        let can_consume = self.leftmost_ref() - self.storage.start;
        self.storage.consume(can_consume);

        if self.storage.should_shift() {
            let amount = self.storage.shift() as u32;
            self.push_left(amount);
        }
    }

    /// Returns how much leading bytes from the HtxBuffer are useless, meaning not referenced by
    /// any Store. It measures how much memory could be saved by shifting the HtxBuffer. It can
    /// be used for monitoring, but it's intended use is internal only.
    pub fn leftmost_ref(&self) -> usize {
        for store in &self.out {
            if let OutBlock::Store(Store::Slice(slice)) = store {
                return slice.start as usize;
            }
        }
        self.storage.head
    }

    #[allow(dead_code)]
    pub fn push_block(&mut self, block: HtxBlock) {
        self.blocks.push_back(block)
    }
    pub fn push_out(&mut self, store: Store) {
        self.out.push_back(OutBlock::Store(store))
    }
    pub fn push_delimiter(&mut self) {
        self.out.push_back(OutBlock::Delimiter)
    }

    #[allow(dead_code)]
    pub fn is_initial(&self) -> bool {
        self.parsing_phase == ParsingPhase::StatusLine
    }

    pub fn is_streaming(&self) -> bool {
        matches!(self.body_size, BodySize::Chunked)
    }

    #[allow(dead_code)]
    pub fn is_main_phase(&self) -> bool {
        match self.parsing_phase {
            ParsingPhase::Body
            | ParsingPhase::Chunks { .. }
            | ParsingPhase::Trailers
            | ParsingPhase::Terminated => true,
            ParsingPhase::StatusLine | ParsingPhase::Headers | ParsingPhase::Error => false,
        }
    }

    pub fn is_error(&self) -> bool {
        self.parsing_phase == ParsingPhase::Error
    }

    pub fn is_terminated(&self) -> bool {
        self.parsing_phase == ParsingPhase::Terminated
    }

    #[allow(dead_code)]
    pub fn is_completed(&self) -> bool {
        self.parsing_phase == ParsingPhase::Terminated
            && self.blocks.is_empty()
            && self.out.is_empty()
    }

    /// Completely reset the Htx state and storage.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.storage.clear();
        self.blocks.clear();
        self.out.clear();
        self.expects = 0;
        self.parsing_phase = ParsingPhase::StatusLine;
        self.body_size = BodySize::Empty;
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsingPhase {
    StatusLine,
    Headers,
    Body,
    /// The "first" field is not directly used by Htx, it is intended for parsers, mainly H1
    /// parsers that can benefit from distinguishing the start of the first chunk from the others.
    Chunks {
        first: bool,
    },
    Trailers,
    Terminated,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodySize {
    Empty,
    Chunked,
    Length(usize),
}

#[derive(Debug)]
pub enum HtxBlock {
    StatusLine(StatusLine),
    Header(Header),
    ChunkHeader(ChunkHeader),
    Chunk(Chunk),
    Flags(Flags),
}

impl HtxBlock {
    pub fn push_left(&mut self, amount: u32) {
        match self {
            HtxBlock::StatusLine(StatusLine::Request {
                method,
                authority,
                path,
                uri,
                ..
            }) => {
                method.push_left(amount);
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
            HtxBlock::ChunkHeader(header) => {
                header.length.push_left(amount);
            }
            HtxBlock::Chunk(chunk) => {
                chunk.data.push_left(amount);
            }
            HtxBlock::Flags(_) => {}
        }
    }
}

#[derive(Debug)]
pub enum StatusLine {
    Request {
        version: Version,
        method: Store,
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
pub struct ChunkHeader {
    pub length: Store,
}

#[derive(Debug)]
pub struct Chunk {
    pub data: Store,
}

#[derive(Debug)]
pub struct Flags {
    pub end_body: bool,
    pub end_chunk: bool,
    pub end_header: bool,
    pub end_stream: bool,
}

#[derive(Debug)]
pub enum OutBlock {
    Delimiter,
    Store(Store),
}

impl OutBlock {
    pub fn push_left(&mut self, amount: u32) {
        match self {
            OutBlock::Store(store) => store.push_left(amount),
            OutBlock::Delimiter => {}
        }
    }

    pub fn consume(self, amount: usize) -> (usize, Option<Store>) {
        match self {
            OutBlock::Store(store) => store.consume(amount),
            OutBlock::Delimiter => (amount, None),
        }
    }
}

#[derive(Debug)]
pub enum Store {
    Empty,
    Slice(Slice),
    #[allow(dead_code)]
    Deported(Slice),
    Static(&'static [u8]),
    Vec(Box<[u8]>, u32),
}

impl Store {
    pub fn new_slice(buffer: &[u8], data: &[u8]) -> Store {
        Store::Slice(Slice::new(buffer, data))
    }

    pub fn new_vec(data: &[u8]) -> Store {
        Store::Vec(data.to_vec().into_boxed_slice(), 0)
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

    pub fn data<'a>(&'a self, buf: &'a [u8]) -> &'a [u8] {
        match self {
            Store::Empty => unreachable!(),
            Store::Slice(slice) | Store::Deported(slice) => slice.data(buf).expect("DATA"),
            Store::Static(data) => data,
            Store::Vec(data, index) => &data[*index as usize..],
        }
    }
    #[allow(dead_code)]
    pub fn data_opt<'a>(&'a self, buf: &'a [u8]) -> Option<&'a [u8]> {
        match self {
            Store::Empty => None,
            Store::Slice(slice) | Store::Deported(slice) => slice.data(buf),
            Store::Static(data) => Some(data),
            Store::Vec(data, index) => Some(&data[*index as usize..]),
        }
    }

    #[allow(dead_code)]
    pub fn modify(&mut self, buf: &mut [u8], new_value: &[u8]) {
        match &self {
            Store::Empty | Store::Deported(_) | Store::Static(_) | Store::Vec(..) => {
                println!("WARNING: modification is not expected on: {self:?}")
            }
            Store::Slice(_) => {}
        }
        match self {
            Store::Empty | Store::Static(_) | Store::Vec(..) => *self = Store::new_vec(new_value),
            Store::Slice(slice) | Store::Deported(slice) => {
                let new_len = new_value.len();
                if slice.len() >= new_len {
                    let start = slice.start as usize;
                    let end = start + new_len;
                    buf[start..end].copy_from_slice(new_value);
                    slice.len = new_len as u32;
                } else {
                    *self = Store::new_vec(new_value)
                }
            }
        }
    }

    fn consume(self, amount: usize) -> (usize, Option<Store>) {
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
                if amount >= data.len() - index as usize {
                    (amount - data.len() + index as usize, None)
                } else {
                    (0, Some(Store::Vec(data, index + amount as u32)))
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
    /// data MUST be a subset of buffer
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

    fn consume(self, amount: usize) -> (usize, Option<Slice>) {
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
