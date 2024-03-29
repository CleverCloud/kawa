use std::fmt::Write;

use crate::storage::{
    AsBuffer, Block, Buffer, Chunk, ChunkHeader, Flags, Kawa, Pair, StatusLine, Store,
};

fn to_utf8(buf: Option<&[u8]>) -> &str {
    match buf {
        Some(buf) => match std::str::from_utf8(buf) {
            Ok(str) => str,
            Err(_) => "[ERROR::UTF8]",
        },
        None => "[ERROR::Kawa]",
    }
}

impl<T: AsBuffer> Kawa<T> {
    pub fn debug(&self, pad: &str) -> Result<String, std::fmt::Error> {
        let buf = self.storage.buffer();
        let mut result = String::new();
        let pad_field = format!("{pad}  ");
        result.write_fmt(format_args!("Kawa {{\n"))?;
        result.write_fmt(format_args!("{pad}  kind: {:?}", self.kind))?;
        result.write_fmt(format_args!(",\n{pad}  buffer: "))?;
        self.storage.debug(&pad_field, &mut result)?;
        result.write_fmt(format_args!(",\n{pad}  expects: {}", self.expects))?;
        result.write_fmt(format_args!(",\n{pad}  phase: {:?}", self.parsing_phase))?;
        result.write_fmt(format_args!(",\n{pad}  body_size: {:?}", self.body_size))?;
        result.write_fmt(format_args!(",\n{pad}  status_line: "))?;
        self.detached
            .status_line
            .debug(buf, &pad_field, &mut result)?;
        let block_pad = format!("{pad}    ");
        result.write_fmt(format_args!(",\n{pad}  jar: ["))?;
        for (i, cookie) in self.detached.jar.iter().enumerate() {
            result.write_fmt(format_args!("\n{block_pad}"))?;
            cookie.debug("Cookie", buf, &block_pad, &mut result)?;
            if i == self.detached.jar.len() - 1 {
                result.write_fmt(format_args!(",\n{pad}  "))?;
            } else {
                result.write_fmt(format_args!(","))?;
            }
        }
        result.write_fmt(format_args!("],\n{pad}  blocks: ["))?;
        for (i, block) in self.blocks.iter().enumerate() {
            result.write_fmt(format_args!("\n{block_pad}"))?;
            match block {
                Block::StatusLine => result.write_fmt(format_args!("StatusLine"))?,
                Block::Cookies => result.write_fmt(format_args!("Cookies"))?,
                Block::Header(block) => block.debug("Header", buf, &block_pad, &mut result)?,
                Block::Chunk(block) => block.debug(buf, &block_pad, &mut result)?,
                Block::ChunkHeader(block) => block.debug(buf, &block_pad, &mut result)?,
                Block::Flags(block) => block.debug(buf, &block_pad, &mut result)?,
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
            match block {
                super::repr::OutBlock::Delimiter => result.write_fmt(format_args!("DELIMITER"))?,
                super::repr::OutBlock::Store(store) => store.debug(buf, &block_pad, &mut result)?,
            }
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
                authority,
                path,
                uri,
            } => {
                result.write_fmt(format_args!("StatusLine::Request {{"))?;
                result.write_fmt(format_args!("\n{pad}  version: {version:?}"))?;
                result.write_fmt(format_args!(",\n{pad}  method: "))?;
                method.debug(buf, &pad_field, result)?;
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
            StatusLine::Unknown => result.write_fmt(format_args!("StatusLine::Unknown"))?,
        }
        Ok(())
    }
}

impl Pair {
    pub fn debug(
        &self,
        name: &str,
        buf: &[u8],
        pad: &str,
        result: &mut String,
    ) -> Result<(), std::fmt::Error> {
        let pad_field = format!("{pad}  ");
        result.write_fmt(format_args!("{name} {{"))?;
        result.write_fmt(format_args!("\n{pad}  key: "))?;
        self.key.debug(buf, &pad_field, result)?;
        result.write_fmt(format_args!(",\n{pad}  val: "))?;
        self.val.debug(buf, &pad_field, result)?;
        result.write_fmt(format_args!(",\n{pad}}}"))?;
        Ok(())
    }
}

impl ChunkHeader {
    pub fn debug(&self, buf: &[u8], pad: &str, result: &mut String) -> Result<(), std::fmt::Error> {
        let pad_field = format!("{pad}  ");
        result.write_fmt(format_args!("ChunkHeader {{"))?;
        result.write_fmt(format_args!("\n{pad}  length: "))?;
        self.length.debug(buf, &pad_field, result)?;
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

impl Flags {
    pub fn debug(&self, _: &[u8], _: &str, result: &mut String) -> Result<(), std::fmt::Error> {
        let flags = [
            (self.end_body, "BODY"),
            (self.end_chunk, "CHUNK"),
            (self.end_header, "HEADER"),
            (self.end_stream, "STREAM"),
        ]
        .into_iter()
        .filter_map(|(flag, name)| if flag { Some(name) } else { None })
        .collect::<Vec<_>>()
        .join("|");
        result.write_fmt(format_args!("Flags ({})", flags))?;
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
                    to_utf8(slice.data_opt(buf))
                ))?;
                result.write_fmt(format_args!(",\n{pad}}}"))?;
            }
            Store::Detached(slice) => {
                result.write_fmt(format_args!("Store::Deported {{"))?;
                result.write_fmt(format_args!("\n{pad}  start: {}", slice.start))?;
                result.write_fmt(format_args!(",\n{pad}  len: {}", slice.len))?;
                result.write_fmt(format_args!(
                    ",\n{pad}  view: {:?}",
                    to_utf8(slice.data_opt(buf))
                ))?;
                result.write_fmt(format_args!(",\n{pad}}}"))?;
            }
            Store::Static(data) => {
                result.write_fmt(format_args!("Store::Static({:?})", to_utf8(Some(data))))?;
            }
            Store::Alloc(data, index) => {
                result.write_fmt(format_args!(
                    "Store::Vec({:?}, {:?})",
                    to_utf8(Some(&data[..*index as usize])),
                    to_utf8(Some(&data[*index as usize..]))
                ))?;
            }
            #[cfg(feature = "rc-alloc")]
            Store::Shared(data, index) => {
                result.write_fmt(format_args!(
                    "Store::Shared({:?}, {:?})",
                    to_utf8(Some(&data[..*index as usize])),
                    to_utf8(Some(&data[*index as usize..]))
                ))?;
            }
        }
        Ok(())
    }
}

impl<T: AsBuffer> Buffer<T> {
    pub fn debug(&self, pad: &str, result: &mut String) -> Result<(), std::fmt::Error> {
        result.write_fmt(format_args!("Buffer {{"))?;
        result.write_fmt(format_args!("\n{pad}  start: {}", self.start))?;
        result.write_fmt(format_args!(",\n{pad}  head: {}", self.head))?;
        result.write_fmt(format_args!(",\n{pad}  end: {}", self.end))?;
        result.write_fmt(format_args!(",\n{pad}  view: {}", self.meter(20)))?;
        result.write_fmt(format_args!(",\n{pad}}}"))?;
        Ok(())
    }
}

pub fn debug_kawa<T: AsBuffer>(kawa: &Kawa<T>) {
    match kawa.debug("") {
        Ok(result) => println!("{result}"),
        Err(error) => println!("{error:?}"),
    }
    // let mut line = String::new();
    // std::io::stdin().read_line(&mut line).expect("stdin");
}
