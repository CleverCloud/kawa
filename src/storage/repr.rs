use std::io::IoSlice;
#[cfg(feature = "rc-alloc")]
use std::rc::Rc;

use crate::storage::{AsBuffer, BlockConverter, Buffer};

#[cfg(feature = "custom-vecdeque")]
use crate::storage::VecDeque;
use log::warn;
#[cfg(not(feature = "custom-vecdeque"))]
use std::collections::VecDeque;

/// Intermediate representation for both H1 and H2 protocols
///
/// /!\ note: the blocks and out fields should always contains "exclusive" data. More specifically
/// out should always contain "older" data than blocks. This is an invariant of the prepare method.
pub struct Kawa<T: AsBuffer> {
    pub storage: Buffer<T>,
    /// Protocol independant representation of the parsed data in the Buffer
    pub blocks: VecDeque<Block>,
    /// Protocol dependant representation generated from the Kawa representation in blocks
    pub out: VecDeque<OutBlock>,

    /// Store the content of specific Blocks away from the "main flow".
    pub detached: DetachedBlocks,

    // Those 4 last fields are set and used by external parsers,
    // Kawa doesn't use them directly.
    pub kind: Kind,
    pub expects: usize,
    pub parsing_phase: ParsingPhase,
    pub body_size: BodySize,

    /// The "consumed" field is not directly used by Kawa, it is intended for proxies, mainly to
    /// easily know if a request started to be transfered. Kawa is responsible for setting it.
    pub consumed: bool,
}

impl<T: AsBuffer> Kawa<T> {
    /// Create a new Kawa struct around a given storage.
    ///
    /// note: the storage is moved into Kawa and shouldn't be directly accessed after that point.
    /// You can retrieve it right before dropping Kawa.
    pub fn new(kind: Kind, storage: Buffer<T>) -> Self {
        Self {
            kind,
            blocks: VecDeque::new(),
            out: VecDeque::new(),
            expects: 0,
            parsing_phase: ParsingPhase::StatusLine,
            body_size: BodySize::Empty,
            storage,
            detached: DetachedBlocks {
                status_line: StatusLine::Unknown,
                jar: VecDeque::new(),
            },
            consumed: false,
        }
    }

    /// Synchronize back all the Stores from out with the underlying data of Buffer.
    /// This is necessary after a Buffer::shift.
    pub fn push_left(&mut self, amount: u32) {
        for block in &mut self.out {
            block.push_left(amount);
        }
    }

    /// Convert Kawa representation from Blocks to a protocol specific representation in out.
    /// BlockConverter takes blocks one by one and should push Stores in the out vector using
    /// dedicated push_out method or push_back on the out field. BlockConverter allows the
    /// implementation of stateful behaviors.
    ///
    /// /!\ note: the interface can seem restrictive, but it enforces some invariants, some that
    /// might not be appearant at first.
    ///
    /// note 2: converters can push delimiters in the out vector (via push_delimiter) to fragment
    /// the "stream". This can be used to split H2 frames.
    pub fn prepare<C: BlockConverter<T>>(&mut self, converter: &mut C) {
        converter.initialize(self);
        while let Some(block) = self.blocks.pop_front() {
            if !converter.call(block, self) {
                break;
            }
        }
        converter.finalize(self);
    }

    /// Return a vector of IoSlices collecting every bytes from the out vector up to its end or a
    /// delimiter: OutBlock::Delimiter. This can be used to split H2 frames.
    ///
    /// note: until you drop the resulting vector, Rust will prevent mutably borrowing Kawa as the
    /// IoSlices keep a reference in the out vector. As always, nothing is copied.
    pub fn as_io_slice(&self) -> Vec<IoSlice> {
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
    /// vector and truncates any partially consumed block. It manages the underlying Buffer,
    /// shifting and synchronizing the data if it deems appropriate.
    ///
    /// note: this function assumes blocks is empty! To respect this invariant you should always
    /// call prepare before consume
    pub fn consume(&mut self, mut amount: usize) {
        // assert!(self.blocks.is_empty());
        // assert!(self.detached.jar.is_empty());
        if amount > 0 {
            self.consumed = true;
        }
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

    /// Returns how much leading bytes from the Buffer are useless, meaning not referenced by
    /// any Store. It measures how much memory could be saved by shifting the Buffer. It can
    /// be used for monitoring, but it's intended use is internal only.
    pub fn leftmost_ref(&self) -> usize {
        for store in &self.out {
            if let OutBlock::Store(Store::Slice(slice)) = store {
                return slice.start as usize;
            }
        }
        if self.blocks.is_empty() {
            // conservative estimate
            self.storage.head
        } else {
            self.storage.start
        }
    }

    pub fn push_block(&mut self, block: Block) {
        self.blocks.push_back(block)
    }
    pub fn push_out(&mut self, store: Store) {
        self.out.push_back(OutBlock::Store(store))
    }
    pub fn push_delimiter(&mut self) {
        self.out.push_back(OutBlock::Delimiter)
    }

    pub fn is_initial(&self) -> bool {
        self.parsing_phase == ParsingPhase::StatusLine
    }

    pub fn is_streaming(&self) -> bool {
        self.body_size == BodySize::Chunked
    }

    pub fn is_main_phase(&self) -> bool {
        match self.parsing_phase {
            ParsingPhase::Body
            | ParsingPhase::Chunks { .. }
            | ParsingPhase::Trailers
            | ParsingPhase::Terminated => true,
            ParsingPhase::StatusLine
            | ParsingPhase::Headers
            | ParsingPhase::Cookies { .. }
            | ParsingPhase::Error { .. } => false,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self.parsing_phase, ParsingPhase::Error { .. })
    }

    pub fn is_terminated(&self) -> bool {
        self.parsing_phase == ParsingPhase::Terminated
    }

    pub fn is_completed(&self) -> bool {
        self.blocks.is_empty() && self.out.is_empty()
    }

    /// Completely reset the Kawa state and storage.
    pub fn clear(&mut self) {
        // self.storage.clear();
        self.blocks.clear();
        self.out.clear();
        self.detached.jar.clear();
        self.detached.status_line = StatusLine::Unknown;
        self.expects = 0;
        self.consumed = false;
        self.parsing_phase = ParsingPhase::StatusLine;
        self.body_size = BodySize::Empty;
    }
}

impl<T: AsBuffer + Clone> Clone for Kawa<T> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            blocks: self.blocks.clone(),
            out: self.out.clone(),
            detached: self.detached.clone(),
            kind: self.kind,
            expects: self.expects,
            parsing_phase: self.parsing_phase,
            body_size: self.body_size,
            consumed: self.consumed,
        }
    }
}

/// Separate the content of the StatusLine and the crumbs from all the cookies from the stream of
/// Blocks. It allows better indexing, persistance and reordering of data. However it is a double
/// edge sword as it currently enables some unwanted/unsafe behavior such as Slice desync and over
/// consuming.
#[derive(Debug, Clone)]
pub struct DetachedBlocks {
    pub status_line: StatusLine,
    pub jar: VecDeque<Pair>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsingPhaseMarker {
    StatusLine,
    Headers,
    Cookies,
    Body,
    Chunks,
    Trailers,
    Terminated,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsingErrorKind {
    Consuming { index: u32 },
    Processing { message: &'static str },
}

impl From<&'static str> for ParsingErrorKind {
    fn from(message: &'static str) -> Self {
        Self::Processing { message }
    }
}
impl From<u32> for ParsingErrorKind {
    fn from(index: u32) -> Self {
        ParsingErrorKind::Consuming { index }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsingPhase {
    StatusLine,
    Headers,
    Cookies {
        first: bool,
    },
    Body,
    /// The "first" field is not directly used by Kawa, it is intended for parsers, mainly H1
    /// parsers that can benefit from distinguishing the start of the first chunk from the others.
    Chunks {
        first: bool,
    },
    Trailers,
    Terminated,
    Error {
        marker: ParsingPhaseMarker,
        kind: ParsingErrorKind,
    },
}

impl ParsingPhase {
    pub fn marker(&self) -> ParsingPhaseMarker {
        match self {
            ParsingPhase::StatusLine => ParsingPhaseMarker::StatusLine,
            ParsingPhase::Headers => ParsingPhaseMarker::Headers,
            ParsingPhase::Cookies { .. } => ParsingPhaseMarker::Cookies,
            ParsingPhase::Body => ParsingPhaseMarker::Body,
            ParsingPhase::Chunks { .. } => ParsingPhaseMarker::Chunks,
            ParsingPhase::Trailers => ParsingPhaseMarker::Trailers,
            ParsingPhase::Terminated => ParsingPhaseMarker::Terminated,
            ParsingPhase::Error { .. } => ParsingPhaseMarker::Error,
        }
    }
    pub fn error(&mut self, kind: ParsingErrorKind) {
        *self = ParsingPhase::Error {
            marker: self.marker(),
            kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodySize {
    Empty,
    Chunked,
    Length(usize),
}

#[derive(Debug, Clone)]
pub enum Block {
    StatusLine,
    Header(Pair),
    Cookies,
    ChunkHeader(ChunkHeader),
    Chunk(Chunk),
    Flags(Flags),
}

impl Block {
    pub fn push_left(&mut self, amount: u32) {
        match self {
            Block::Header(header) => {
                header.key.push_left(amount);
                header.val.push_left(amount);
            }
            Block::ChunkHeader(header) => {
                header.length.push_left(amount);
            }
            Block::Chunk(chunk) => {
                chunk.data.push_left(amount);
            }
            Block::StatusLine | Block::Cookies | Block::Flags(_) => {}
        }
    }
}

#[derive(Debug, Clone)]
pub enum StatusLine {
    Unknown,
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

impl StatusLine {
    pub fn pop(&mut self) -> StatusLine {
        match self {
            StatusLine::Request { version, .. } => {
                let mut owned = StatusLine::Request {
                    version: *version,
                    method: Store::Empty,
                    authority: Store::Empty,
                    path: Store::Empty,
                    uri: Store::Empty,
                };
                std::mem::swap(self, &mut owned);
                owned
            }
            StatusLine::Response { version, code, .. } => {
                let mut owned = StatusLine::Response {
                    version: *version,
                    code: *code,
                    status: Store::Empty,
                    reason: Store::Empty,
                };
                std::mem::swap(self, &mut owned);
                owned
            }
            StatusLine::Unknown => StatusLine::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pair {
    pub key: Store,
    pub val: Store,
}

impl Pair {
    pub fn elide(&mut self) {
        self.key = Store::Empty;
    }

    pub fn is_elided(&self) -> bool {
        self.key.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ChunkHeader {
    pub length: Store,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub data: Store,
}

#[derive(Debug, Clone)]
pub struct Flags {
    pub end_body: bool,
    pub end_chunk: bool,
    pub end_header: bool,
    pub end_stream: bool,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub enum Store {
    Empty,
    Slice(Slice),
    Detached(Slice),
    Static(&'static [u8]),
    Alloc(Box<[u8]>, u32),
    #[cfg(feature = "rc-alloc")]
    Shared(Rc<[u8]>, u32),
}

impl Store {
    pub fn new_slice(buffer: &[u8], data: &[u8]) -> Store {
        Store::Slice(Slice::new(buffer, data))
    }

    pub fn new_detached(buffer: &[u8], data: &[u8]) -> Store {
        Store::Detached(Slice::new(buffer, data))
    }

    pub fn from_vec(data: Vec<u8>) -> Store {
        Store::Alloc(data.into_boxed_slice(), 0)
    }

    pub fn from_slice(data: &[u8]) -> Store {
        Store::Alloc(data.to_vec().into_boxed_slice(), 0)
    }

    pub fn from_string(data: String) -> Store {
        Store::Alloc(data.into_bytes().into_boxed_slice(), 0)
    }

    pub fn push_left(&mut self, amount: u32) {
        match self {
            Store::Slice(slice) => {
                slice.start -= amount;
            }
            Store::Detached(slice) => {
                slice.start -= amount;
            }
            _ => {}
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Store::Empty => 0,
            Store::Slice(s) | Store::Detached(s) => s.len(),
            Store::Static(s) => s.len(),
            Store::Alloc(s, i) => s.len() - *i as usize,
            #[cfg(feature = "rc-alloc")]
            Store::Shared(s, i) => s.len() - *i as usize,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Store::Empty)
    }

    pub fn data<'a>(&'a self, buf: &'a [u8]) -> &'a [u8] {
        match self {
            Store::Empty => unreachable!(),
            Store::Slice(slice) | Store::Detached(slice) => slice.data(buf),
            Store::Static(data) => data,
            Store::Alloc(data, index) => &data[*index as usize..],
            #[cfg(feature = "rc-alloc")]
            Store::Shared(data, index) => &data[*index as usize..],
        }
    }
    pub fn data_opt<'a>(&'a self, buf: &'a [u8]) -> Option<&'a [u8]> {
        match self {
            Store::Empty => None,
            Store::Slice(slice) | Store::Detached(slice) => slice.data_opt(buf),
            Store::Static(data) => Some(data),
            Store::Alloc(data, index) => Some(&data[*index as usize..]),
            #[cfg(feature = "rc-alloc")]
            Store::Shared(data, index) => Some(&data[*index as usize..]),
        }
    }

    pub fn capture(self, buf: &[u8]) -> Store {
        match self {
            Store::Slice(slice) | Store::Detached(slice) => Store::from_slice(slice.data(buf)),
            _ => self,
        }
    }

    pub fn modify(&mut self, buf: &mut [u8], new_value: &[u8]) {
        match self {
            Store::Slice(slice) | Store::Detached(slice) => {
                let new_len = new_value.len();
                if slice.len() >= new_len {
                    let start = slice.start as usize;
                    let end = start + new_len;
                    buf[start..end].copy_from_slice(new_value);
                    slice.len = new_len as u32;
                } else {
                    *self = Store::from_slice(new_value)
                }
            }
            _ => {
                warn!("modification is not expected on: {self:?}");
                *self = Store::from_slice(new_value)
            }
        }
    }

    pub fn split(self, at: usize) -> (Store, Store) {
        let at32 = at as u32;
        match self {
            Store::Empty => (Store::Empty, Store::Empty),
            Store::Slice(Slice { start, len }) => (
                Store::Slice(Slice { start, len: at32 }),
                Store::Slice(Slice {
                    start: start + at32,
                    len: len - at32,
                }),
            ),
            Store::Detached(Slice { start, len }) => (
                Store::Detached(Slice { start, len: at32 }),
                Store::Detached(Slice {
                    start: start + at32,
                    len: len - at32,
                }),
            ),
            Store::Static(s) => (Store::Static(&s[..at]), Store::Static(&s[at..])),
            Store::Alloc(s, i) => (
                Store::from_slice(&s[i as usize..i as usize + at]),
                Store::Alloc(s, i + at32),
            ),
            #[cfg(feature = "rc-alloc")]
            Store::Shared(s, i) => (
                Store::from_slice(&s[i as usize..i as usize + at]),
                Store::Shared(s, i + at32),
            ),
        }
    }

    fn consume(self, amount: usize) -> (usize, Option<Store>) {
        match self {
            Store::Empty => (amount, None),
            Store::Slice(slice) => {
                let (remaining, opt) = slice.consume(amount);
                (remaining, opt.map(Store::Slice))
            }
            Store::Detached(slice) => {
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
            Store::Alloc(data, index) => {
                if amount >= data.len() - index as usize {
                    (amount - data.len() + index as usize, None)
                } else {
                    (0, Some(Store::Alloc(data, index + amount as u32)))
                }
            }
            #[cfg(feature = "rc-alloc")]
            Store::Shared(data, index) => {
                if amount >= data.len() - index as usize {
                    (amount - data.len() + index as usize, None)
                } else {
                    (0, Some(Store::Shared(data, index + amount as u32)))
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
        // assert!(
        //     offset <= u32::MAX as usize,
        //     "slices should not start at more than 4GB from its beginning"
        // );
        // assert!(
        //     data.len() <= u16::MAX as usize,
        //     "slices should not be larger than 65536 bytes"
        // );
        Slice {
            start: offset as u32,
            len: data.len() as u32,
        }
    }

    pub fn data<'a>(&self, buffer: &'a [u8]) -> &'a [u8] {
        let start = self.start as usize;
        let end = start + self.len();
        &buffer[start..end]
    }

    pub fn data_opt<'a>(&self, buffer: &'a [u8]) -> Option<&'a [u8]> {
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

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Version {
    Unknown,
    V10,
    V11,
    V20,
}
