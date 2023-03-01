use std::{fmt::Write, io::IoSlice};

/// Intermediate representation for both H1 and H2 protocols
/// ```txt
/// H2
/// [
///     [:pseudo] \
///     [:pseudo] |- SL
///     [:pseudo] /
///     [K:V]
///     [K:V]
///     +K:V+
///     -K:V-
///     [K:V]
///     [CHUNK]
///     [CHUNK]
///     [CHUNK]
///     [CHUNK]
///     [K:V]
///     [K:V]
/// ]
///
/// H1
/// [
///     [SL]
///     [K:V]
///     [K:V]
///     +K:V+
///     -K:V-
///     [K:V]
///     [CHUNK]
///     [CHUNK]
///     [CHUNK]
///     [CHUNK]
///     [K:V]
///     [K:V]
/// ]
/// ```
#[derive(Debug)]
pub struct HTX {
    pub kind: HtxKind,
    pub blocks: Vec<HtxBlock>,
    pub out: Vec<Store>,
    /// the start of the unparsed area in the buffer
    pub index: usize,
    pub expects: usize,
    pub parsing_phase: HtxParsingPhase,
    pub body_size: HtxBodySize,
}

impl HTX {
    pub fn new(kind: HtxKind) -> Self {
        Self {
            kind,
            blocks: Vec::new(),
            out: Vec::new(),
            index: 0,
            expects: 0,
            parsing_phase: HtxParsingPhase::StatusLine,
            body_size: HtxBodySize::Empty,
        }
    }
    pub fn new_request() -> Self {
        Self::new(HtxKind::Request)
    }
    pub fn new_response() -> Self {
        Self::new(HtxKind::Response)
    }

    pub fn push_left(&mut self, amount: u32) {
        if amount == 0 {
            return;
        }
        self.index -= amount as usize;
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

    pub fn as_io_slice<'a>(&'a mut self, buf: &'a [u8]) -> Vec<IoSlice<'a>> {
        self.out
            .iter()
            .map(|store| IoSlice::new(store.data(buf).expect("DATA")))
            .collect()
    }

    pub fn consume(&mut self, mut amount: usize) -> usize {
        let mut push_right = 0;
        let mut stores_left = Vec::new();
        let mut iter = self.out.drain(..);
        while let Some(store) = iter.next() {
            let (remaining, advance, store) = store.consume(amount);
            amount = remaining;
            match advance {
                Some(amount) => push_right = amount,
                None => {}
            }
            match store {
                Some(store) => stores_left.push(store),
                None => {}
            }
        }
        stores_left.extend(iter);
        self.out = stores_left;

        assert_eq!(amount, 0);
        push_right
    }

    pub fn leftmost_ref(&self) -> usize {
        for store in &self.out {
            match store {
                Store::Slice(slice) => {
                    return slice.start as usize;
                }
                _ => {}
            }
        }
        self.index
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
    pub fn new_slice(buffer: &[u8], data: &[u8]) -> Self {
        Self::Slice(Slice::new(buffer, data))
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

    pub fn data<'a>(&'a self, buffer: &'a [u8]) -> Option<&'a [u8]> {
        match self {
            Store::Empty => None,
            Store::Slice(slice) | Store::Deported(slice) => slice.data(buffer),
            Store::Static(data) => Some(data),
            Store::Vec(data, index) => Some(&data[*index..]),
        }
    }

    pub fn modify(&mut self, buffer: &mut [u8], new_value: &[u8]) {
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
                if slice.len as usize >= new_len {
                    let start = slice.start as usize;
                    let end = start + new_len;
                    buffer[start..end].copy_from_slice(new_value);
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

    pub fn consume(self, amount: usize) -> (usize, Option<usize>, Option<Store>) {
        match self {
            Store::Empty => (amount, None, None),
            Store::Slice(slice) => {
                let (remaining, push_right, opt) = slice.consume(amount);
                (remaining, Some(push_right), opt.map(Store::Slice))
            }
            Store::Deported(slice) => {
                let (remaining, _, opt) = slice.consume(amount);
                (remaining, None, opt.map(Store::Slice))
            }
            Store::Static(data) => {
                if amount >= data.len() {
                    (amount - data.len(), None, None)
                } else {
                    (0, None, Some(Store::Static(&data[amount..])))
                }
            }
            Store::Vec(data, index) => {
                if amount >= data.len() - index {
                    (amount - data.len() + index, None, None)
                } else {
                    (0, None, Some(Store::Vec(data, index + amount)))
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
        let end = start + self.len as usize;

        if start <= buffer.len() && end <= buffer.len() {
            Some(&buffer[start..end])
        } else {
            None
        }
    }

    pub fn consume(self, amount: usize) -> (usize, usize, Option<Slice>) {
        if amount >= self.len as usize {
            (amount - self.len(), self.end(), None)
        } else {
            let Slice { start, len } = self;
            (
                0,
                self.end() - amount,
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
    pub fn end(&self) -> usize {
        (self.start + self.len) as usize
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Version {
    V10,
    V11,
    V20,
}

fn to_utf8(buf: Option<&[u8]>) -> &str {
    match buf {
        Some(buf) => match std::str::from_utf8(buf) {
            Ok(str) => str,
            Err(_) => "[ERROR::UTF8]",
        },
        None => "[ERROR::HTX]",
    }
}

impl HTX {
    pub fn debug(&self, buf: &[u8], pad: &str) -> Result<String, std::fmt::Error> {
        let mut result = String::new();
        result.write_fmt(format_args!("HTX {{\n"))?;
        result.write_fmt(format_args!("{pad}  kind: {:?},\n", self.kind))?;
        result.write_fmt(format_args!("{pad}  index: {},\n", self.index))?;
        result.write_fmt(format_args!("{pad}  expects: {},\n", self.expects))?;
        result.write_fmt(format_args!(
            "{pad}  parsing_phase: {:?},\n",
            self.parsing_phase
        ))?;
        result.write_fmt(format_args!("{pad}  body_size: {:?},\n", self.body_size))?;
        result.write_fmt(format_args!("{pad}  blocks: ["))?;
        let block_pad = format!("{pad}    ");
        for (i, block) in self.blocks.iter().enumerate() {
            result.write_fmt(format_args!("\n{block_pad}"))?;
            match block {
                HtxBlock::StatusLine(block) => block.debug(buf, &block_pad, &mut result)?,
                HtxBlock::Header(block) => block.debug(buf, &block_pad, &mut result)?,
                HtxBlock::Chunk(block) => block.debug(buf, &block_pad, &mut result)?,
            }
            if i == self.blocks.len() - 1 {
                result.write_fmt(format_args!(",\n{pad}  "))?;
            } else {
                result.write_fmt(format_args!(","))?;
            }
        }
        result.write_fmt(format_args!("],\n{pad}  out: ["))?;
        let block_pad = format!("{pad}    ");
        for (i, block) in self.out.iter().enumerate() {
            result.write_fmt(format_args!("\n{block_pad}"))?;
            block.debug(buf, &block_pad, &mut result)?;
            if i == self.out.len() - 1 {
                result.write_fmt(format_args!(",\n{pad}  "))?;
            } else {
                result.write_fmt(format_args!(","))?;
            }
        }
        result.write_fmt(format_args!("],\n{pad}}}"))?;

        Ok(result)
    }
}

impl StatusLine {
    pub fn debug(&self, buf: &[u8], pad: &str, result: &mut String) -> Result<(), std::fmt::Error> {
        let pad_field = format!("{pad}  ");
        match &self {
            StatusLine::Request {
                version,
                method,
                scheme,
                authority,
                path,
                uri,
            } => {
                result.write_fmt(format_args!("StatusLine::Request {{"))?;
                result.write_fmt(format_args!("\n{pad}  version: {version:?}"))?;
                result.write_fmt(format_args!(",\n{pad}  method: "))?;
                method.debug(buf, &pad_field, result)?;
                result.write_fmt(format_args!(",\n{pad}  scheme: "))?;
                scheme.debug(buf, &pad_field, result)?;
                result.write_fmt(format_args!(",\n{pad}  authority: "))?;
                authority.debug(buf, &pad_field, result)?;
                result.write_fmt(format_args!(",\n{pad}  path: "))?;
                path.debug(buf, &pad_field, result)?;
                result.write_fmt(format_args!(",\n{pad}  uri: "))?;
                uri.debug(buf, &pad_field, result)?;
                result.write_fmt(format_args!(",\n{pad}}}"))?;
            }
            StatusLine::Response {
                version,
                code,
                status,
                reason,
            } => {
                result.write_fmt(format_args!("StatusLine::Response {{"))?;
                result.write_fmt(format_args!("\n{pad}  version: {version:?}"))?;
                result.write_fmt(format_args!(",\n{pad}  code: {code}"))?;
                result.write_fmt(format_args!(",\n{pad}  status: "))?;
                status.debug(buf, &pad_field, result)?;
                result.write_fmt(format_args!(",\n{pad}  reason: "))?;
                reason.debug(buf, &pad_field, result)?;
                result.write_fmt(format_args!(",\n{pad}}}"))?;
            }
        }
        Ok(())
    }
}
impl Header {
    pub fn debug(&self, buf: &[u8], pad: &str, result: &mut String) -> Result<(), std::fmt::Error> {
        let pad_field = format!("{pad}  ");
        result.write_fmt(format_args!("Header {{"))?;
        result.write_fmt(format_args!("\n{pad}  key: "))?;
        self.key.debug(buf, &pad_field, result)?;
        result.write_fmt(format_args!(",\n{pad}  val: "))?;
        self.val.debug(buf, &pad_field, result)?;
        result.write_fmt(format_args!(",\n{pad}}}"))?;
        Ok(())
    }
}
impl Chunk {
    pub fn debug(&self, buf: &[u8], pad: &str, result: &mut String) -> Result<(), std::fmt::Error> {
        let pad_field = format!("{pad}  ");
        result.write_fmt(format_args!("Chunk {{"))?;
        result.write_fmt(format_args!("\n{pad}  data: "))?;
        self.data.debug(buf, &pad_field, result)?;
        result.write_fmt(format_args!(",\n{pad}}}"))?;
        Ok(())
    }
}
impl Store {
    pub fn debug(&self, buf: &[u8], pad: &str, result: &mut String) -> Result<(), std::fmt::Error> {
        match self {
            Store::Empty => {
                result.write_fmt(format_args!("Store::Empty"))?;
            }
            Store::Slice(slice) => {
                result.write_fmt(format_args!("Store::Slice {{"))?;
                result.write_fmt(format_args!("\n{pad}  start: {}", slice.start))?;
                result.write_fmt(format_args!(",\n{pad}  len: {}", slice.len))?;
                result.write_fmt(format_args!(
                    ",\n{pad}  view: {:?}",
                    to_utf8(slice.data(buf))
                ))?;
                result.write_fmt(format_args!(",\n{pad}}}"))?;
            }
            Store::Deported(slice) => {
                result.write_fmt(format_args!("Store::Deported {{"))?;
                result.write_fmt(format_args!("\n{pad}  start: {}", slice.start))?;
                result.write_fmt(format_args!(",\n{pad}  len: {}", slice.len))?;
                result.write_fmt(format_args!(
                    ",\n{pad}  view: {:?}",
                    to_utf8(slice.data(buf))
                ))?;
                result.write_fmt(format_args!(",\n{pad}}}"))?;
            }
            Store::Static(data) => {
                result.write_fmt(format_args!("Store::Static({:?})", to_utf8(Some(data))))?;
            }
            Store::Vec(data, index) => {
                result.write_fmt(format_args!(
                    "Store::Vec({:?}, {:?})",
                    to_utf8(Some(&data[..*index])),
                    to_utf8(Some(&data[*index..]))
                ))?;
            }
        }
        Ok(())
    }
}

pub fn debug_htx(htx: &HTX, buf: &[u8]) {
    match htx.debug(buf, "") {
        Ok(result) => println!("{result}"),
        Err(error) => println!("{error:?}"),
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).expect("stdin");
}
